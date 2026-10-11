#!/usr/bin/env python3
"""Machine-level control of "midget" (Lenovo ThinkCentre M900 on titan's eno2, 10.99.0.0/24) from titan.

usage: tools/midget.py idle LABEL [--seconds N] | mem | apps | recovery | status | bios [list|get N|set N V|find WORD|dump] | me | labboot on|off | wake | reboot [--netboot|--disk] | poweroff | bootnext pxe|disk|NNNN | efi | hw | screen OUT.png | state | ota | wait NAME --value V | ax [PREFIX] | press NAME | key enter|esc|tab|space|backspace|up|down|left|right|cmd+o | tap-window APP X Y | tap X Y | context X Y | hover X Y | open-url URL | open-app ID | open-folder ID PATH | show-in-files PATH | swipe X0 Y0 X1 Y1 [MS] | trace on|off | update [--restart] | appearance light|dark|auto | wallpaper ID | logs [N] | amt | ssh CMD... | reinstall --confirm \"REINSTALL midget\" [--timeout S] | wipe --disk DEV --confirm "WIPE 10.99.0.200 DEV" [--dry-run]
Coordinates: `screen` writes compositor pixels. `tap`/`swipe`/`context`/`hover` take the shell's own geometry (what
`state` prints as window=APP,X,Y,W,H), which can be offset from the screenshot because the compositor places windows
itself (seen 2026-10-10: Files at 557,187 in `state`, 680,307 in the PNG). For anything inside a window use
`tap-window APP X Y` with window-local pixels (APP as `state` names it: Settings, AppStore, External(N)); for a
full-size app window (manifest window = "full-size") the window-local origin is the top-left corner, with no title bar.
Reaches a live lab ISO and, because the lab installer leaves the lab key, an installed system. Power-on of a switched-off machine is Wake-on-LAN
(needs it enabled in the BIOS) or, if the board has Intel AMT, `amt`. Every call is bounded; nothing here erases a disk.
Stdlib only (PIL optional for `screen` on an installed system).
"""
from pathlib import Path
import json, os, re, shlex, socket, struct, subprocess, sys, time, urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def _reachable_host():
    """VLAN 20 first, then the lab NIC (eno2) and Wi-Fi: the first that answers ssh within a second."""
    import socket
    for h in ("192.168.20.140", "10.99.0.200", "192.168.1.187"):
        try:
            socket.create_connection((h, 22), 1).close()
            return h
        except OSError:
            pass
    return "192.168.20.140"


HOST = os.environ.get("MIDGET_HOST") or _reachable_host()
MAC = os.environ.get("MIDGET_MAC", "00:23:24:dc:c8:40")  # iVentoy's client id 0023-24dc-c840
BCAST = "10.99.0.255"
def _lab_key():
    """out/lab is untracked: in a git worktree the key lives in the main checkout, so fall back to its out/lab."""
    local = os.path.join(REPO, "out/lab/id_ed25519")
    if os.path.exists(local):
        return local
    common = subprocess.run(["git", "-C", REPO, "rev-parse", "--git-common-dir"], capture_output=True, text=True).stdout.strip()
    main = os.path.dirname(os.path.abspath(os.path.join(REPO, common))) if common else ""
    shared = os.path.join(main, "out/lab/id_ed25519")
    return shared if main and os.path.exists(shared) else local


KEY = _lab_key()
SSH = ["ssh", "-i", KEY, "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "LogLevel=ERROR", f"root@{HOST}"]


def run(cmd, timeout=30):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=timeout)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, 124, "", "timeout")


def ssh(cmd, timeout=30):
    return run(SSH + [cmd], timeout).stdout


def ping():
    return run(["ping", "-c1", "-W1", HOST], 5).returncode == 0


def port_open(port, timeout=1.0):
    try:
        socket.create_connection((HOST, port), timeout).close()
        return True
    except OSError:
        return False


def state():
    """down | up (pings, no usable ssh) | live | installed"""
    if not ping():
        return "down"
    out = ssh("test -d /run/archiso && echo live || echo installed", 8).strip()
    return out if out in ("live", "installed") else "up"


def wait(pred, timeout, step=1.0):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return False


def efi_entries():
    return ssh("efibootmgr", 15)


def pxe_entry(text):
    rows = [l for l in text.splitlines() if l.startswith("Boot") and ("PXE" in l or "IPv4" in l)]
    rows.sort(key=lambda l: ("IPv4" not in l, l))
    return rows[0][4:8] if rows else None


def disk_entry(text):
    rows = [l for l in text.splitlines() if l.startswith("Boot") and "BifrOSt" in l]
    return rows[0][4:8] if rows else None


def iventoy_unattended():
    """True when iVentoy boots a default lab image after a timeout, so a netboot needs no keypress."""
    try:
        req = urllib.request.Request("http://127.0.0.1:26000/iventoy/json", data=b'{"method":"get_config_all"}',
                                     headers={"Content-Type": "application/json"})
        cfg = json.load(urllib.request.urlopen(req, timeout=5))
    except Exception:
        return False
    return bool(cfg.get("defimg", "").startswith("iso/bifrost-lab")) and int(cfg.get("deftimeout", 0)) > 0


def cmd_status():
    st = state()
    print(f"state: {st}   host {HOST}   mac {MAC}")
    print("ports: " + " ".join(f"{p}={'open' if port_open(p) else 'closed'}" for p in (22, 16992, 16993, 623, 5900)))
    if st in ("live", "installed"):
        print(ssh("echo uptime=$(cut -d' ' -f1 /proc/uptime)s; echo kernel=$(uname -r); echo boot=$(cat /proc/cmdline | tr ' ' '\\n' | grep -E '^(root|archisobasedir|initrd)=' | head -3 | tr '\\n' ' '); "
                  "echo springboard=$(systemctl is-active springboard 2>&1); echo installer=$(systemctl is-active bifrost-installer 2>&1)"))
    log = os.path.join(REPO, "out/iventoy/iventoy-1.0.44/log/log.txt")
    if os.path.exists(log):
        last = [l.strip() for l in open(log, errors="replace").read().splitlines() if "DHCP" in l or "iSCSI" in l][-2:]
        print("iventoy: " + " | ".join(l[:110] for l in last))
    return 0


def cmd_wake():
    magic = b"\xff" * 6 + bytes(int(x, 16) for x in MAC.split(":")) * 16
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    for _ in range(3):
        s.sendto(magic, (BCAST, 9))
        time.sleep(0.2)
    print(f"magic packet sent to {MAC} via {BCAST}; waiting up to 90 s for it to answer")
    ok = wait(ping, 90, 2)
    print("answers ping" if ok else "no answer (Wake-on-LAN may be off in the BIOS, or the NIC lost power)")
    return 0 if ok else 1


def schedule_power(action):
    # detached so ssh returns before the machine goes away; --force --force = no unit teardown (a netbooted root would hang)
    verb = {"reboot": "reboot", "poweroff": "poweroff"}[action]
    ssh(f"nohup sh -c 'sleep 1; sync; systemctl {verb} --force --force' >/dev/null 2>&1 &", 15)


def cmd_bootnext(which):
    text = efi_entries()
    num = which if len(which) == 4 and all(c in "0123456789ABCDEFabcdef" for c in which) else (pxe_entry(text) if which == "pxe" else disk_entry(text))
    if not num:
        print("no such boot entry:\n" + text)
        return 1
    ssh(f"efibootmgr -n {num} >/dev/null")
    print(f"BootNext={num}")
    return 0


def cmd_reboot(args):
    current = state()
    if current == "down":
        print("midget is not reachable over ssh; use `wake` if it is off, or power-cycle it by hand")
        return 1
    if current == "up":
        # answers ping but ssh is not usable yet (booting or probation): wait for it instead of giving up
        print("midget answers ping but ssh is not ready; waiting")
        if not wait(lambda: state() in ("live", "installed"), 300, 2):
            print("did not come back in 5 min")
            return 1
        cmd_recovery()
        return 0
    if "--netboot" in args:
        if cmd_bootnext("pxe"):
            return 1
    elif "--disk" in args:
        if cmd_bootnext("disk"):
            return 1
    schedule_power("reboot")
    t = time.time()
    wait(lambda: not ping(), 60, 1)
    print(f"went down after {time.time() - t:.0f}s; waiting for it to come back")
    ok = wait(lambda: state() in ("live", "installed"), 300, 2)
    print(f"back as {state()} after {time.time() - t:.0f}s" if ok else "did not come back in 5 min")
    if ok:
        cmd_recovery()
    return 0 if ok else 1


def cmd_poweroff():
    schedule_power("poweroff")
    ok = wait(lambda: not ping(), 60, 1)
    print("off" if ok else "still answering")
    return 0 if ok else 1


def cmd_hw():
    print(ssh("echo '--- machine'; cat /sys/class/dmi/id/{sys_vendor,product_name,product_version,bios_version,bios_date} 2>/dev/null | paste -sd' '; "
              "echo '--- cpu/mem'; lscpu | grep -E 'Model name|^CPU\\(s\\)'; free -h | sed -n 2p; "
              "echo '--- disks'; lsblk -dno NAME,SIZE,MODEL,TRAN; "
              "echo '--- gpu'; lspci -nn | grep -Ei 'vga|display'; ls /dev/dri 2>/dev/null | tr '\\n' ' '; echo; "
              "echo '--- nics'; ip -br link | grep -v lo; "
              "echo '--- temps'; cat /sys/class/thermal/thermal_zone*/temp 2>/dev/null | tr '\\n' ' '; echo; "
              "echo '--- uefi'; [ -d /sys/firmware/efi ] && echo uefi || echo bios", 40))
    return 0


def cmd_screen(out):
    """Installer frame hook on a live system; the shell's control port on an installed one."""
    st = state()
    if st == "live":
        ssh("touch /run/bifrost-shot-on")
        time.sleep(1.2)
        data = subprocess.run(SSH + ["cat /run/bifrost-screen.png"], capture_output=True, timeout=30).stdout
        if data[:4] == b"\x89PNG":
            open(out, "wb").write(data)
            print(out)
            return 0
    elif st == "installed":
        capture = subprocess.run(SSH + ["test -S /run/bifrost/user/wayland-0 && bifrost-apps --screenshot"], capture_output=True, timeout=30)
        if capture.returncode == 0 and capture.stdout.startswith(b"\x89PNG"):
            open(out, "wb").write(capture.stdout)
            from PIL import Image
            with Image.open(out) as img: w,h = img.size
            print(f"{out} ({w}x{h} compositor pixels)")
            return 0
        script = "exec 3<>/dev/tcp/127.0.0.1/7777; echo 'shot 1' >&3; timeout 20 cat <&3"
        blob = subprocess.run(SSH + [f"bash -c {json.dumps(script)}"], capture_output=True, timeout=40).stdout
        if blob[:4] == b"RIMG":
            w, h = struct.unpack("<II", blob[4:12])
            from PIL import Image
            Image.frombuffer("RGBA", (w, h), blob[12:12 + w * h * 4], "raw", "BGRA", 0, 1).convert("RGB").save(out)
            print(f"{out} ({w}x{h} panel pixels)")
            return 0
    print(f"no screen grab possible (state {st})")
    return 1


def cmd_show_in_files(args):
    """Show in Files exactly as an app does it: bifrost-apps --open <folder>, run as the local account."""
    if len(args) != 1 or not args[0].startswith("/"):
        print("usage: show-in-files /absolute/folder")
        return 2
    script = ("user=$(sed -n 's/^username=//p' /var/lib/springboard/account); "
              "runuser -u \"$user\" -- /usr/bin/bifrost-apps --open " + shlex.quote(args[0]))
    r = run(SSH + [script], 20)
    print(r.stdout, end="")
    if r.stderr:
        print(r.stderr, end="", file=sys.stderr)
    return r.returncode


def cmd_control(args):
    """Send full-panel pixel input/state requests through the loopback control socket over SSH."""
    line = " ".join(args)
    if not line or "\n" in line or len(line) > 500:
        print("invalid control command")
        return 2
    script = ("import socket; s=socket.create_connection(('127.0.0.1',7777),5); "
              "s.settimeout(10); s.sendall(" + repr((line + "\n").encode()) + "); "
              "d=b''\nwhile True:\n c=s.recv(65536)\n if not c: break\n d+=c\nprint(d.decode(),end='')")
    r = run(SSH + ["python3 -c " + shlex.quote(script)], 15)
    print(r.stdout, end="")
    if r.stderr:
        print(r.stderr, end="", file=sys.stderr)
    if not r.returncode and args[0] == "tap":
        time.sleep(0.12)  # The control response acknowledges a queued gesture; wait for its release.
    return r.returncode or (1 if r.stdout.startswith("error:") else 0)


# evdev codes for `key`: named keys, modifiers, letters and digits
KEY_CODES = {"enter": 28, "esc": 1, "tab": 15, "space": 57, "backspace": 14,
             "up": 103, "down": 108, "left": 105, "right": 106,
             "cmd": 125, "super": 125, "ctrl": 29, "shift": 42, "alt": 56,
             **{c: n for c, n in zip("qwertyuiop", range(16, 26))},
             **{c: n for c, n in zip("asdfghjkl", range(30, 39))},
             **{c: n for c, n in zip("zxcvbnm", range(44, 51))},
             **{c: n for c, n in zip("1234567890", range(2, 12))}}


def key_codes(spec):
    """`enter`, `o` or a combination `cmd+shift+s` -> evdev codes in press order; None when a part is unknown."""
    parts = spec.lower().split("+")
    if not spec or any(p not in KEY_CODES for p in parts):
        return None
    return [KEY_CODES[p] for p in parts]


def cmd_key(args):
    """Inject a real compositor keyboard key or combination (`cmd+o`) through a temporary uinput device."""
    codes = key_codes(args[0]) if len(args) == 1 else None
    if not codes:
        print("usage: key NAME or MOD+KEY (names: " + " ".join(sorted(KEY_CODES)) + ")")
        return 2
    script = """import os,fcntl,struct,time
codes=CODES
fd=os.open('/dev/uinput',os.O_WRONLY|os.O_NONBLOCK)
def ev(code,value):
 os.write(fd,struct.pack('qqHHi',0,0,1,code,value))
 os.write(fd,struct.pack('qqHHi',0,0,0,0,0))
 time.sleep(.05)
try:
 fcntl.ioctl(fd,0x40045564,1)
 for c in codes:
  fcntl.ioctl(fd,0x40045565,c)
 os.write(fd,struct.pack('80sHHHHI',b'BifrOSt remote keyboard',3,1,1,1,0)+bytes(1024))
 fcntl.ioctl(fd,0x5501)
 time.sleep(.3)
 for c in codes:
  ev(c,1)
 for c in reversed(codes):
  ev(c,0)
 fcntl.ioctl(fd,0x5502)
finally:
 os.close(fd)
""".replace("CODES", repr(codes))
    result = run(SSH + ["python3 -c " + shlex.quote(script)], 10)
    print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    return result.returncode


def cmd_window_tap(args):
    """Tap client-local pixels, independent of the compositor's initial placement."""
    if len(args) != 3:
        print("usage: tap-window AppStore|External(N)|Settings X Y (window-local pixels; see `state` for names)")
        return 2
    try:
        x, y = float(args[1]), float(args[2])
    except ValueError:
        return 2
    result = ssh("python3 -c " + shlex.quote("import socket; s=socket.create_connection(('127.0.0.1',7777),5); s.settimeout(5); s.sendall(b'state\\n'); print(s.recv(65536).decode())"), 10)
    for line in result.splitlines():
        fields = line.split('window=', 1)[-1].split(',')
        if 'window=' in line and len(fields) == 5 and fields[0].lower() == args[0].lower():
            if not (0 <= x < int(fields[3]) and 0 <= y < int(fields[4])):
                print('Coordinates outside the window')
                return 2
            return cmd_control(['tap',str(float(fields[1])+x),str(float(fields[2])+y)])
    print('Window not open: ' + args[0])
    return 1


def cmd_update(args):
    """Install a guarded system transaction; apply/reboot when --restart is selected."""
    if any(a not in ("--restart",) for a in args):
        print("usage: update [--restart]")
        return 2
    adopt = "/usr/bin/springboard,/usr/lib/systemd/system/springboard.service,/usr/lib/springboard/seed-data"
    # A system update already staged by an earlier `update` is applied, not installed again:
    # the guard refuses a second install while one is pending.
    status = run(SSH + ["bifrost-update-guard status"], 30).stdout
    staged = "phase=system-prepared" in status
    if staged:
        print("update already staged; applying it")
    else:
        r = run(SSH + ["bifrost-update-guard install"], 600)
        print(r.stdout, end="")
        print(r.stderr, end="", file=sys.stderr)
        if r.returncode:
            return r.returncode
    if "--restart" in args:
        r = run(SSH + ["bifrost-update-guard apply"], 30)
        print(r.stderr, end="", file=sys.stderr)
        return r.returncode
    print("Installed. Run update --restart to apply it (Btrfs system updates reboot).")
    return 0


def cmd_logs(n):
    d = os.path.join(REPO, "out/lab/logs")
    logs = sorted(f for f in os.listdir(d) if f.endswith(".log")) if os.path.isdir(d) else []
    if not logs:
        print("no lab logs")
        return 1
    print(subprocess.run(["tail", "-n", str(n), os.path.join(d, logs[-1])], capture_output=True, text=True, errors="replace").stdout)
    return 0


BIOS_BASE = "/sys/class/firmware-attributes/thinklmi"


def cmd_bios(args):
    """BIOS settings through the kernel's think-lmi driver (Lenovo WMI): read, search, change and save from the OS, no keyboard in the BIOS.
    A supervisor password, if one is set, goes in $BIOS_PASSWORD (never stored). Changes apply at the next boot."""
    op = args[0] if args else "list"
    if state() not in ("live", "installed"):
        print(f"midget is {state()}: bios needs ssh (boot it into the lab ISO first: `bifrost pc midget wake`, or netboot it by hand once)")
        return 1
    probe = f"modprobe think-lmi 2>/dev/null; [ -d {BIOS_BASE}/attributes ] || {{ echo NOLMI; ls /sys/bus/wmi/devices 2>/dev/null | head -20; dmidecode -s bios-version 2>/dev/null; exit 3; }}"
    if op in ("list", "dump", "find"):
        out = ssh(probe + f"; cd {BIOS_BASE}/attributes; for a in *; do [ -f $a/current_value ] || continue; "
                  "echo \"$a=$(cat $a/current_value)|$(cat $a/possible_values 2>/dev/null)\"; done", 60)
        if out.startswith("NOLMI"):
            print("think-lmi is not available on this kernel/BIOS (the WMI interface is missing). WMI devices seen:\n" + out[6:])
            print("Fallbacks: efibootmgr (`efi`, `bootnext`), Wake-on-LAN, AMT (`amt`), or the BIOS menu by hand (F1).")
            return 1
        rows = [l for l in out.splitlines() if "=" in l]
        if op == "find":
            rows = [l for l in rows if args[1].lower() in l.lower()] if len(args) > 1 else rows
        if op == "dump":
            d = os.path.join(REPO, "out/lab/bios")
            os.makedirs(d, exist_ok=True)
            path = os.path.join(d, time.strftime("midget-%Y%m%d-%H%M%S.json"))
            json.dump({l.split("=", 1)[0]: l.split("=", 1)[1] for l in rows}, open(path, "w"), indent=1, sort_keys=True)
            print(f"{len(rows)} settings -> {path}")
            return 0
        print("\n".join(rows) or "no attributes")
        return 0
    if op == "get" and len(args) > 1:
        print(ssh(probe + f"; cat {BIOS_BASE}/attributes/{args[1]}/current_value; cat {BIOS_BASE}/attributes/{args[1]}/possible_values 2>/dev/null"))
        return 0
    if op == "set" and len(args) > 2:
        name, value = args[1], args[2]
        pw = os.environ.get("BIOS_PASSWORD", "")
        auth = f"echo '{pw}' > {BIOS_BASE}/authentication/Admin/current_password; " if pw else ""
        out = ssh(probe + f"; {auth}echo '{value}' > {BIOS_BASE}/attributes/{name}/current_value && echo save > {BIOS_BASE}/attributes/save_settings && echo saved; "
                  f"cat {BIOS_BASE}/attributes/{name}/current_value", 40)
        print(out)
        return 0 if "saved" in out else 1
    print("usage: bios list | find WORD | get NAME | set NAME VALUE | dump")
    return 2


def cmd_me():
    """What the OS can tell about Intel ME/AMT without the network ports: the M900 is vPro-certified, but many units ship Standard Manageability (ISM)
    or no AMT at all (modded ME firmware exists for exactly that reason), so check before trusting remote power/KVM."""
    if state() not in ("live", "installed"):
        print(f"midget is {state()}: needs ssh")
        return 1
    print(ssh("echo '--- pci'; lspci -nn | grep -iE 'management engine|mei|heci'; "
              "echo '--- mei device'; ls -l /dev/mei* 2>&1 | head -2; cat /sys/class/mei/mei0/fw_ver 2>/dev/null; cat /sys/class/mei/mei0/fw_status 2>/dev/null; "
              "echo '--- dmesg'; dmesg | grep -iE 'mei|heci|amt' | head -8; "
              "echo '--- tools'; for t in amtinfo meshcmd lms; do command -v $t || echo \"$t: not installed\"; done; "
              "echo '--- bios'; dmidecode -s system-product-name -s bios-version -s bios-release-date 2>/dev/null | paste -sd' '", 40))
    print("\nIf an ME device is there, `meshcmd amtinfo` (MeshCentral's meshcmd, run on midget) tells the AMT/ISM state and version. "
          "Provisioning needs MEBx once at the keyboard (Ctrl+P at boot): set the password, enable Network Access and (Un)Configure.")
    return 0


def cmd_labboot(args):
    """`labboot on`: UEFI BootOrder = PXE first, so a reset or power cycle always lands in the live lab ISO (rescue without a keyboard);
    the reinstall loop sets BootNext=disk before it reboots into an installed system. `labboot off`: the BifrOSt disk first again."""
    on = (args[0] if args else "on") == "on"
    text = efi_entries()
    pxe, disk = pxe_entry(text), disk_entry(text)
    order = next((l.split(":", 1)[1].strip().split(",") for l in text.splitlines() if l.startswith("BootOrder:")), [])
    if not pxe:
        print("no PXE entry:\n" + text)
        return 1
    first = [pxe] if on else ([disk] if disk else [])
    new = first + [e for e in order if e not in first]
    ssh(f"efibootmgr -o {','.join(new)} >/dev/null")
    print(f"BootOrder: {','.join(new)}  ({'lab: PXE first' if on else 'normal: disk first'})")
    return 0


# --- driving by name (docs/agent-api.md): closed-loop steps instead of sleeps and coordinates ---------------------------------

def control_text(line, timeout=15):
    """One request on the shell's control socket; None when ssh or the socket does not answer."""
    script = ("import socket; s=socket.create_connection(('127.0.0.1',7777),5); s.settimeout(10); "
              "s.sendall(" + repr((line + "\n").encode()) + "); d=b''\nwhile True:\n c=s.recv(65536)\n if not c: break\n d+=c\nprint(d.decode(),end='')")
    try:
        r = run(SSH + ["python3 -c " + shlex.quote(script)], timeout)
    except Exception:
        return None
    return r.stdout if r.returncode == 0 else None


def ax_rows(prefix=""):
    """The element map as dicts (address, role, name, value, enabled); None when the shell does not answer."""
    out = control_text(("ax list " + prefix).strip())
    if out is None or out.startswith("error"):
        return None
    rows = []
    for line in out.splitlines():
        p = line.split("\t")
        if len(p) >= 5:
            rows.append({"address": p[0], "role": p[1], "name": p[2], "value": p[3], "enabled": p[4] == "enabled"})
    return rows


def ax_find(rows, query):
    """The row with this exact address, else the unique one whose address ends with /query."""
    for r in rows or []:
        if r["address"] == query:
            return r
    hits = [r for r in rows or [] if r["address"].endswith("/" + query)]
    return hits[0] if len(hits) == 1 else None


def ax_press(query):
    out = (control_text("ax press " + query) or "no answer").strip()
    return out.startswith("ok"), out


def shell_screen():
    out = control_text("state") or ""
    return next((l.split("=", 1)[1] for l in out.splitlines() if l.startswith("screen=")), None)


def cmd_wait(args):
    """wait NAME [--value V] [--gone] [--enabled] [--timeout S]: block until the element shows up (with that value), or is gone."""
    if not args:
        print("usage: wait NAME [--value V] [--gone] [--enabled] [--timeout S]")
        return 2
    query, value, gone, enabled, timeout = args[0], None, False, False, 30.0
    rest = args[1:]
    while rest:
        a = rest.pop(0)
        if a == "--value" and rest:
            value = rest.pop(0)
        elif a == "--timeout" and rest:
            timeout = float(rest.pop(0))
        elif a == "--gone":
            gone = True
        elif a == "--enabled":
            enabled = True
        else:
            print(f"unknown option {a}")
            return 2
    found = {}

    def ready():
        rows = ax_rows()
        if rows is None:
            return False
        r = ax_find(rows, query)
        if gone:
            return r is None
        if r is None or (value is not None and r["value"] != value) or (enabled and not r["enabled"]):
            return False
        found.update(r)
        return True

    if wait(ready, timeout, 1):
        print("gone" if gone else f"{found['address']}\t{found['value']}")
        return 0
    print(f"timed out after {timeout:.0f}s waiting for {query}" + (" to go" if gone else ""))
    return 1


def cmd_ota(args):
    """Update BifrOSt through Settings by name: check, download, restart, unlock, and confirm the guard accepted it."""
    def step(msg):
        print(f"[ota] {msg}", flush=True)

    def press(query):
        ok, out = ax_press(query)
        if not ok:
            step(f"press {query}: {out}")
        return ok

    def alert(confirm):
        # the confirmation sheet: its second button is named after the action (download, restart, check)
        if not wait(lambda: any(r["address"] == "os/settings/alert" for r in ax_rows("os/settings/alert") or []), 15, 0.5):
            step("no confirmation sheet appeared")
            return False
        rows = ax_rows("os/settings/alert") or []
        buttons = [r for r in rows if r["role"] == "button" and not r["address"].endswith("/cancel")]
        return bool(buttons) and press(buttons[0]["address"] if confirm else "os/settings/alert/cancel")

    if shell_screen() == "lock":
        step("unlocking")
        cmd_key(["enter"])
        wait(lambda: shell_screen() not in (None, "lock"), 20, 1)
    before = (control_text("state") or "").split("version=")[-1].split()[0:1]
    if not press("os/apps/settings") or not press("General > Software Update"):
        return 1
    page = "os/settings/general/software-update"
    deadline = time.time() + float(next((a.split("=")[1] for a in args if a.startswith("--timeout=")), 900))
    asked = False
    saw_checking = False
    while time.time() < deadline:
        rows = ax_rows(page) or []
        have = {r["address"].rsplit("/", 1)[-1] for r in rows}
        if "checking-for-update" in have:
            # opening the page starts a check of its own: let it finish rather than press a row that is about to go
            saw_checking = True
            time.sleep(2)
            continue
        if "install-now" in have:
            step("update staged; restarting to install")
            if not (press(page + "/install-now") and alert(True)):
                return 1
            break
        if "download-and-install" in have:
            step("update available; downloading")
            if not (press(page + "/download-and-install") and alert(True)):
                return 1
            # the page can lag behind the device: a prepared system also counts
            t0 = time.time()
            while time.time() - t0 < 420:
                have = {r["address"].rsplit("/", 1)[-1] for r in ax_rows(page) or []}
                if "install-now" in have:
                    break
                if "phase=system-prepared" in (run(SSH + ["bifrost-update-guard status"]).stdout or ""):
                    # prepared is set before pacman runs: rebooting during mkinitcpio boots a truncated initramfs
                    step("system prepared; waiting for pacman to finish")
                    staged = "test -f /boot/EFI/BIFROST/pending && grep -qx 0 /boot/EFI/BIFROST/pending && ! test -e /var/lib/pacman/db.lck"
                    if not wait(lambda: run(SSH + [staged]).returncode == 0, 600, 2):
                        step("pacman did not finish; not restarting")
                        return 1
                    step("system staged on the device; restarting")
                    return finish_ota(before, reboot_first=True)
                time.sleep(3)
            continue
        if "check-for-update" in have and not asked and not saw_checking:
            step("checking for an update")
            asked = True
            ok, out = ax_press(page + "/check-for-update")
            if not ok:
                step(f"retrying: {out}")  # the page changed state between listing and pressing
                asked = False
                time.sleep(2)
                continue
            if not alert(True):
                return 1
            wait(lambda: {r["address"].rsplit("/", 1)[-1] for r in ax_rows(page) or []} & {"download-and-install", "install-now"}, 45, 2)
            continue
        if "check-for-update" in have and (asked or saw_checking):
            step("up to date; nothing to install")
            return 0
        time.sleep(2)
    else:
        step("timed out")
        return 1
    return finish_ota(before)


def finish_ota(before, reboot_first=False):
    def step(msg):
        print(f"[ota] {msg}", flush=True)
    if reboot_first:
        schedule_power("reboot")
    wait(lambda: not ping(), 120, 1)
    step("restarting; waiting for it to come back")
    if not wait(lambda: state() in ("live", "installed"), 420, 2):
        step("did not come back in 7 min")
        return 1
    wait(lambda: shell_screen() is not None, 120, 2)
    if shell_screen() == "lock":
        cmd_key(["enter"])
    ok = wait(lambda: "phase=accepted" in (run(SSH + ["bifrost-update-guard status"]).stdout or ""), 150, 3)
    out = control_text("state") or ""
    version = next((l.split("=", 1)[1] for l in out.splitlines() if l.startswith("version=")), "?")
    step(f"version {version}, guard {'accepted' if ok else 'NOT accepted yet'}")
    return 0 if ok else 1


def cmd_recovery():
    result = run(SSH + ["bifrost-update-guard status"])
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    return result.returncode


def cmd_apps():
    result = run(SSH + ["cat /run/bifrost/app-resources.json"])
    if result.returncode:
        print(result.stderr, end="", file=sys.stderr)
        return result.returncode
    try:
        print(json.dumps(json.loads(result.stdout), indent=2))
    except ValueError:
        print("Invalid app resource snapshot", file=sys.stderr)
        return 1
    return 0


def cmd_memory():
    """System RAM and process-tree PSS without double-counting shared memory."""
    script = Path(__file__).with_name("midget-memory.py").read_text()
    result = subprocess.run(SSH + ["python3 -"], input=script, text=True, capture_output=True, timeout=30)
    print(result.stdout, end="")
    if result.stderr: print(result.stderr, end="", file=sys.stderr)
    return result.returncode


def cmd_amt():
    """Intel AMT/vPro (remote power, boot-to-PXE, KVM) answers on 16992/16993 (web), 623 (RMCP) when enabled in MEBx."""
    found = {p: port_open(p) for p in (16992, 16993, 623, 664)}
    print("AMT ports:", {str(p): ("open" if v else "closed") for p, v in found.items()})
    if not any(found.values()):
        print("no AMT. The M900 supports vPro only on some SKUs; enable it in MEBx (Ctrl+P at boot) and give it its own IP or share the OS NIC. "
              "Until then: Wake-on-LAN (`wake`) for power-on, ssh for everything else.")
        return 1
    print(f"AMT web UI: http://{HOST}:16992 (user admin); power control via wsman once a password is set")
    return 0


def set_pref(key, value):
    """Writes one settings.ini key on the Midget. The shell keeps settings.ini in memory and rewrites it, so the
    file is changed while the shell is stopped; the shell then starts and is unlocked. Returns True when the file and
    the running shell both report the value."""
    script = (
        "set -e; systemctl stop springboard; f=/var/lib/springboard/settings.ini; "
        "python3 - \"$f\" " + shlex.quote(key) + " " + shlex.quote(value) + " <<'PY'\n"
        "import os, sys\n"
        "path, key, value = sys.argv[1], sys.argv[2], sys.argv[3]\n"
        "lines = open(path).read().splitlines() if os.path.exists(path) else []\n"
        "lines = [l for l in lines if not l.startswith(key + '=')]\n"
        "lines.append(key + '=' + value)\n"
        "tmp = path + '.tmp'\n"
        "open(tmp, 'w').write('\\n'.join(lines) + '\\n')\n"
        "os.replace(tmp, path)\n"
        "PY\n"
        "systemctl start springboard; sleep 3; systemctl is-active springboard; "
        "grep '^" + key + "=' \"$f\""
    )
    r = run(SSH + [script], 90)
    out = (r.stdout + r.stderr).strip()
    print(out)
    ok = r.returncode == 0 and key + "=" + value in out and "active" in out
    if ok:
        # the restarted shell opens on the lock screen: unlock it and wait for the desktop
        cmd_key(["enter"])
        deadline = time.time() + 30
        while time.time() < deadline and shell_screen() not in ("home", "app:settings"):
            time.sleep(1)
        print("screen=" + str(shell_screen()))
    return ok


def cmd_appearance(args):
    """appearance light|dark|auto: sets Displays > Appearance on the Midget live (older shells: restart, unlock, check)."""
    if len(args) != 1 or args[0] not in ("light", "dark", "auto"):
        sys.exit("usage: midget appearance light|dark|auto")
    out = (control_text("appearance " + args[0]) or "").strip()
    if out == "ok":
        print("appearance=" + args[0])
        sys.exit(0)
    # shells before 0.4.91 have no live command: write the pref with a restart
    sys.exit(0 if set_pref("appearance_mode", args[0]) else 1)


def cmd_wallpaper(args):
    """wallpaper ID: sets the wallpaper on the Midget to a packed still (`/var/lib/springboard/wallpapers/ID.raw`),
    e.g. svartifoss_basalt. Restarts the shell like `appearance`; the file must exist on the device."""
    if len(args) != 1 or not re.fullmatch(r"[a-z0-9_]+", args[0]):
        sys.exit("usage: midget wallpaper ID   (a packed still id, e.g. svartifoss_basalt)")
    have = run(SSH + ["test -f /var/lib/springboard/wallpapers/" + args[0] + ".raw && echo yes"], 20).stdout.strip()
    if have != "yes":
        sys.exit("no wallpaper " + args[0] + " on the device (see /var/lib/springboard/wallpapers/*.raw)")
    sys.exit(0 if set_pref("wallpaper", args[0]) else 1)


# --- autonomous reinstall (docs/bifrost-pc-workflow.md "Reinstall Midget autonomously") ---------------------------------------

def _stamp(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def lab_password():
    """The tester account's lab password: out/lab/tester.password (untracked, 0600), made once. It is published to the netbooted
    installer as out/lab/sync/lab/tester.pw on titan's lab HTTP server (10.99.0.1:8078), never into the repo or the preset."""
    labdir = os.path.dirname(KEY)
    pw_file = os.path.join(labdir, "tester.password")
    if not os.path.exists(pw_file):
        import secrets
        fd = os.open(pw_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(secrets.token_urlsafe(12) + "\n")
    pw = open(pw_file).read().strip()
    pub = os.path.join(labdir, "sync", "lab")
    os.makedirs(pub, exist_ok=True)
    with open(os.path.join(pub, "tester.pw"), "w") as f:
        f.write(pw + "\n")
    return pw, pw_file


def ensure_lab_http():
    if run(["ss", "-ltn"], 5).stdout.find(":8078 ") >= 0:
        return True
    sync = os.path.join(os.path.dirname(KEY), "sync")
    if not os.path.exists(os.path.join(sync, "bifrost-installer")):
        return False
    subprocess.Popen(["python3", "-m", "http.server", "8078", "--bind", "10.99.0.1"], cwd=sync, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    time.sleep(1)
    return True


def mcp_call(tool, args, timeout=30):
    """One bifrost-mcp tool call over ssh (the same binary the agents use; no token needed with the lab key)."""
    return ssh(f"bifrost-mcp call {tool} {shlex.quote(json.dumps(args))} 2>&1", timeout)


def cmd_reinstall(args):
    """`reinstall --confirm "REINSTALL midget" [--timeout S]`: MCP `lifecycle reinstall`, then follow the phases with timestamps."""
    confirm = args[args.index("--confirm") + 1] if "--confirm" in args and args.index("--confirm") + 1 < len(args) else ""
    budget = int(args[args.index("--timeout") + 1]) if "--timeout" in args else 2400
    if state() != "installed":
        print(f"midget is {state()}; reinstall starts from an installed system (use `reboot --netboot` for a live one)")
        return 1
    host = ssh("cat /proc/sys/kernel/hostname", 10).strip()
    if confirm != f"REINSTALL {host}":
        print(f'refused: pass --confirm "REINSTALL {host}"')
        return 2
    if not iventoy_unattended():
        print("iVentoy would stop at its boot menu (no default image or timeout 0): run tools/lab-up.sh first")
        return 1
    if not ensure_lab_http():
        print("lab HTTP server (10.99.0.1:8078) is not up and out/lab/sync has no installer; run tools/lab-push.sh first")
        return 1
    _, pw_file = lab_password()
    _stamp(f"tester password: {pw_file} (published to the lab server for the installer only)")
    t0 = time.time()
    out = mcp_call("lifecycle", {"action": "reinstall", "confirm": confirm})
    _stamp("lifecycle reinstall: " + out.strip().replace("\n", " | ")[:300])
    if "job=" not in out:
        return 1
    def left():
        return budget - (time.time() - t0)
    if not wait(lambda: not ping(), 60, 1):
        _stamp("FAILED: midget never went down")
        return 1
    _stamp("phase 1/7 going down: rebooting")
    if not wait(lambda: state() == "live", min(600, left()), 3):
        _stamp("FAILED: no netboot into the live lab ISO (check iVentoy and BootNext with `efi`)")
        return 1
    _stamp("phase 2/7 netboot seen: live lab ISO is up")
    seen = False
    while left() > 0 and state() == "live":
        j = ssh("journalctl -u bifrost-installer --no-pager -n 400 2>/dev/null | grep -E 'unattended|bifrost-installer: \\[' | tail -n 1", 15).strip()
        if j and not seen:
            seen = True
            _stamp("phase 3/7 installer running: " + j[-140:])
        elif j:
            print("    " + j[-150:], flush=True)
        if "refused or failed" in j:
            _stamp("FAILED: " + j)
            return 1
        time.sleep(20)
    if state() != "installed" and not wait(lambda: state() == "installed", left(), 3):
        _stamp("FAILED: the installed system did not answer on ssh in time")
        return 1
    if not seen:
        _stamp("note: never saw installer progress lines; the install may have run between polls")
    _stamp("phase 4/7 installed system up (ssh with the lab key)")
    # a fresh install has no update in flight: the guard reports an empty phase; only an update leaves one to accept
    def booted_ok():
        g = ssh("bifrost-update-guard status", 15) or ""
        return ("phase=accepted" in g or "phase=\n" in g + "\n") and "ready=1" in (control_text("state") or "")
    if not wait(booted_ok, min(300, left()), 5):
        _stamp("FAILED: the new system did not boot cleanly (update guard phase, shell ready)")
        return 1
    _stamp("phase 5/7 phase=accepted")
    def tester_up():
        return bool(ssh("getent passwd tester >/dev/null && ps -u tester -o comm= | head -n 1", 15).strip())
    def locked():
        return "lock" in (control_text("state", 15) or "").lower().split("screen")[-1][:40]
    ok = wait(tester_up, min(180, left()), 5)
    if ok and locked():
        cmd_key(["enter"])
        time.sleep(3)
    _stamp("phase 6/7 tester session " + ("running" if ok else "NOT running"))
    mcp = wait(lambda: port_open(7420) and "host=" in mcp_call("system", {"what": "overview"}, 20), min(120, left()), 5)
    _stamp("phase 7/7 MCP " + ("reachable on :7420 and answering" if mcp else "NOT reachable"))
    _stamp(f"done in {time.time() - t0:.0f}s: " + ("OK" if ok and mcp else "INCOMPLETE"))
    return 0 if ok and mcp else 1


def main():
    a = sys.argv[1:] or ["status"]
    op, rest = a[0], a[1:]
    if op in ("-h", "--help", "help"):
        print(__doc__)
        return
    table = {
        "status": lambda: cmd_status(),
        "mem": lambda: cmd_memory(),
        "apps": lambda: cmd_apps(),
        "recovery": lambda: cmd_recovery(),
        "idle": lambda: subprocess.call([sys.executable, str(Path(__file__).with_name("midget-idle-measure.py")), *rest]),
        "reinstall": lambda: cmd_reinstall(rest),
        "wipe": lambda: subprocess.call([sys.executable, str(Path(__file__).with_name("midget-wipe.py")), *rest]),
        "state": lambda: cmd_control(["state"]),
        "lock": lambda: cmd_control(["lock"]),
        # the element map (docs/agent-api.md): ax [prefix] lists, press NAME presses by address, path or unique name
        "ax": lambda: cmd_control(["ax", "list"] + rest),
        "press": lambda: cmd_control(["ax", "press"] + rest),
        "wait": lambda: cmd_wait(rest),
        "ota": lambda: cmd_ota(rest),
        "appearance": lambda: cmd_appearance(rest),
        "wallpaper": lambda: cmd_wallpaper(rest),
        "context":lambda:cmd_control(["context"]+rest),
        "hover":lambda:cmd_control(["hover"]+rest),
        "open-url":lambda:cmd_control(["open-url"]+rest),
        "open-app":lambda:cmd_control(["open-app"]+rest),
        # the shell opens the app (default Files) at a folder through the control socket
        "open-folder":lambda:cmd_control(["open-folder"]+rest),
        "show-in-files":lambda:cmd_show_in_files(rest),
        "tap": lambda: cmd_control(["tap"] + rest),
        "tap-window": lambda: cmd_window_tap(rest),
        "key": lambda: cmd_key(rest),
        "swipe": lambda: cmd_control(["swipe"] + rest),
        "trace": lambda: cmd_control(["trace"] + rest),
        "update": lambda: cmd_update(rest),
        "wake": lambda: cmd_wake(),
        "reboot": lambda: cmd_reboot(rest),
        "poweroff": lambda: cmd_poweroff(),
        "bootnext": lambda: cmd_bootnext(rest[0] if rest else "pxe"),
        "efi": lambda: (print(efi_entries()), 0)[1],
        "hw": lambda: cmd_hw(),
        "screen": lambda: cmd_screen(rest[0] if rest else "/tmp/midget.png"),
        "logs": lambda: cmd_logs(int(rest[0]) if rest else 40),
        "amt": lambda: cmd_amt(),
        "labboot": lambda: cmd_labboot(rest),
        "me": lambda: cmd_me(),
        "bios": lambda: cmd_bios(rest),
        "ssh": lambda: (print(ssh(" ".join(rest), 120), end=""), 0)[1],
    }
    if op not in table:
        sys.exit(__doc__)
    sys.exit(table[op]())


if __name__ == "__main__":
    main()
