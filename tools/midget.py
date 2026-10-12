#!/usr/bin/env python3
"""Machine-level control of "midget" (Lenovo ThinkCentre M900 on titan's eno2, 10.99.0.0/24) from titan.

usage: tools/midget.py idle LABEL [--seconds N] | kbench LABEL [--seconds N] | kab [--seconds N] | amt-info | amt-activate | power state|on|off|cycle|reset | mem | apps | recovery | unlock | login-bypass on|off | status | bios [list|get N|set N V|find WORD|dump] | me | labboot on|off | wake | reboot [--netboot|--disk] | poweroff | bootnext pxe|disk|NNNN | efi | hw | screen OUT.png | state | ota | wait NAME --value V | ax [PREFIX] | press NAME | key enter|esc|tab|space|backspace|up|down|left|right|cmd+o | tap-window APP X Y | tap X Y | context X Y | hover X Y | open-url URL | open-app ID | open-folder ID PATH | show-in-files PATH | swipe X0 Y0 X1 Y1 [MS] | trace on|off | update [--restart] | appearance light|dark|auto | wallpaper ID | logs [N] | amt | ssh CMD... | qa-install [--disk DEV] [--shots DIR] [--timeout S] | reinstall --confirm \"REINSTALL midget\" [--timeout S] [--shots S] [--loop N] | follow [--timeout S] [--shots S] | wipe --disk DEV --confirm "WIPE 10.99.0.200 DEV" [--dry-run]
Coordinates: `screen` writes compositor pixels. `tap`/`swipe`/`context`/`hover` take the shell's own geometry (what
`state` prints as window=APP,X,Y,W,H), which can be offset from the screenshot because the compositor places windows
itself (seen 2026-10-10: Files at 557,187 in `state`, 680,307 in the PNG). For anything inside a window use
`tap-window APP X Y` with window-local pixels (APP as `state` names it: Settings, AppStore, External(N)); for a
full-size app window (manifest window = "full-size") the window-local origin is the top-left corner, with no title bar.
Reaches a live lab ISO and, because the lab installer leaves the lab key, an installed system. Power-on of a switched-off machine is Wake-on-LAN
(needs it enabled in the BIOS) or, if the board has Intel AMT, `amt`. Every call is bounded; nothing here erases a disk.
Stdlib only (PIL optional for `screen` on an installed system).
Host: $MIDGET_HOST, else the first of 192.168.20.140 (VLAN 20), 10.99.0.200 (lab NIC), 192.168.1.187 (Wi-Fi) whose ssh answers
(all probed at once, 1.5 s); with none answering it uses the lab NIC 10.99.0.200.
`kbench`/`kab` measure kernel latency (tools/midget/kbench.sh, docs/audio-latency.md "Kernel"); `amt-info`/`amt-activate`/`power`
are Intel AMT (docs/bifrost-pc-workflow.md "AMT findings"); the AMT password is only ever read from out/lab/amt.password.
"""
from pathlib import Path
import contextlib, io, json, os, re, shlex, socket, struct, subprocess, sys, time, urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HOST_ORDER = ("192.168.20.140", "10.99.0.200", "192.168.1.187")  # VLAN 20, lab NIC (eno2), Wi-Fi


def pick_host(answered, order=HOST_ORDER):
    """First address of `order` in `answered` (a set of addresses whose ssh port connected), else the lab NIC:
    the lab NIC is the link that is always wired, so it is the fallback when nothing answers (VLAN 20 can be down)."""
    return next((h for h in order if h in answered), "10.99.0.200")


def _reachable_host(order=HOST_ORDER, timeout=1.5):
    """Probe every address at once (a dead one cannot delay the rest) and take the first in `order` that opens ssh."""
    import concurrent.futures

    def probe(h):
        try:
            socket.create_connection((h, 22), timeout).close()
            return h
        except OSError:
            return None
    with concurrent.futures.ThreadPoolExecutor(len(order)) as pool:
        return pick_host({h for h in pool.map(probe, order) if h}, order)


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


def run(cmd, timeout=30, input=None):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=timeout, input=input)
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


# US layout: unshifted characters and what Shift adds, as evdev codes
_SHIFTED = {"!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0"}
_PUNCT = {"-": 12, "=": 13, "[": 26, "]": 27, ";": 39, "'": 40, "`": 41, "\\": 43, ",": 51, ".": 52, "/": 53}
_PUNCT_SHIFT = {"_": "-", "+": "=", "{": "[", "}": "]", ":": ";", '"': "'", "~": "`", "|": "\\", "<": ",", ">": ".", "?": "/"}


def text_keys(text):
    """Password text -> [(shift, evdev code)] for a US keyboard; None for a character without a key."""
    seq = []
    for ch in text:
        low = ch.lower()
        if ch == " ":
            seq.append((False, 57))
        elif low in KEY_CODES and len(low) == 1:
            seq.append((ch.isupper(), KEY_CODES[low]))
        elif ch in _SHIFTED:
            seq.append((True, KEY_CODES[_SHIFTED[ch]]))
        elif ch in _PUNCT:
            seq.append((False, _PUNCT[ch]))
        elif ch in _PUNCT_SHIFT:
            seq.append((True, _PUNCT[_PUNCT_SHIFT[ch]]))
        else:
            return None
    return seq


_RUST_CHAR = r"'(\\u\{[0-9a-fA-F]+\}|\\.|[^'\\])'"


def _rust_char(lit):
    if lit.startswith("\\u{"):
        return chr(int(lit[3:-1], 16))
    if lit.startswith("\\"):
        return {"0": "\0", "n": "\n", "t": "\t"}.get(lit[1], lit[1])
    return lit


def installer_keymaps(path=None):
    """The installer's own keyboard tables (springboard/src/installer/keymaps_data.rs): (layout ids in order,
    per layout {code: [plain, shift, altgr, shift+altgr]}, {(mark, base): composed})."""
    text = open(path or os.path.join(REPO, "springboard/src/installer/keymaps_data.rs"), encoding="utf-8").read()
    lay = text[text.index("pub const LAYOUTS"):text.index("pub const KEYMAPS")]
    ids = re.findall(r'^\s*\("([^"]+)",', lay, re.M)
    body = text[text.index("pub const KEYMAPS"):text.index("pub const COMPOSE")]
    maps = []
    for block in re.split(r"^\s*// .*$", body, flags=re.M)[1:]:
        m = {}
        for code, chars in re.findall(r"\((\d+), \[(.*?)\]\)", block):
            cs = [_rust_char(c) for c in re.findall(_RUST_CHAR, chars)]
            if len(cs) == 4:
                m[int(code)] = cs
        maps.append(m)
    comp = {}
    for a, b, c in re.findall(r"\(" + _RUST_CHAR + ", " + _RUST_CHAR + ", " + _RUST_CHAR + r"\)", text[text.index("pub const COMPOSE"):]):
        comp[(_rust_char(a), _rust_char(b))] = _rust_char(c)
    return ids, maps[:len(ids)], comp


def layout_keys(text, layout="us", tables=None):
    """Text -> [(shift, code, altgr)] key presses that type it on the installer's `layout` (dead key then base letter for
    accented ones, as a user would); None when a character has no key there."""
    ids, maps, comp = tables or installer_keymaps()
    if layout not in ids:
        return None
    km = maps[ids.index(layout)]
    direct = {}
    for code in sorted(km):
        for level, c in enumerate(km[code]):
            if c != "\0" and (c not in direct or level < direct[c][0]):
                direct[c] = (level, code)
    press = lambda c: (direct[c][0] & 1 == 1, direct[c][1], direct[c][0] >= 2)
    seq = []
    for ch in text:
        if ch == " ":
            seq.append((False, 57, False))
        elif ch in direct and not "\u0300" <= ch <= "\u036f":
            seq.append(press(ch))
        else:
            pair = next(((m, b) for (m, b), c in comp.items() if c == ch and m in direct and b in direct), None)
            if pair is None:
                return None
            seq += [press(pair[0]), press(pair[1])]
    return seq


def lab_keyboard():
    """The keyboard layout `qa-install` picks on the installer's Language screen: `keyboard=` (an installer layout id such as
    `us` or `is`) in out/lab/account.conf, else `us`."""
    conf = os.path.join(os.path.dirname(KEY), "account.conf")
    if os.path.exists(conf):
        for line in open(conf, encoding="utf-8"):
            key, _, value = line.strip().partition("=")
            if key == "keyboard" and re.fullmatch(r"[a-z-]+", value):
                return value
    return "us"


def lab_account():
    """The account a `reinstall` creates: `username=` and `full_name=` lines in out/lab/account.conf (untracked), else
    the lab default tester / Tester. Its password lives in out/lab/<username>.password (untracked, mode 0600)."""
    user, name = "tester", "Tester"
    conf = os.path.join(os.path.dirname(KEY), "account.conf")
    if os.path.exists(conf):
        for line in open(conf, encoding="utf-8"):
            key, _, value = line.strip().partition("=")
            if key == "username" and re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", value):
                user = value
            elif key == "full_name" and value and '"' not in value and "\\" not in value:
                name = value
    return user, name


def login_password():
    """The password `unlock` types: ~/.config/bifrost/midget-login (yours, mode 600) when present, else the lab account's
    password (out/lab/<username>.password, see `lab_account`). Never printed or logged."""
    path = Path.home() / ".config/bifrost/midget-login"
    if path.exists():
        st = path.stat()
        if st.st_uid != os.getuid() or st.st_mode & 0o077:
            raise SystemExit(f"{path} must be owned by you and mode 600 (chmod 600 {path}); refusing to read it")
        return path.read_text().rstrip("\n")
    pw_file = os.path.join(os.path.dirname(KEY), f"{lab_account()[0]}.password")
    return open(pw_file).read().strip() if os.path.exists(pw_file) else None


def cmd_unlock():
    """Type the login password on the login window and press Enter, through a temporary uinput keyboard (an older shell
    without the login window unlocks on the Enter alone)."""
    password = login_password()
    if password is None:
        print("no lab password: run a reinstall, write ~/.config/bifrost/midget-login (chmod 600), or `login-bypass on`")
        return 2
    seq = text_keys(password)
    if seq is None:
        print("the lab password has a character `unlock` cannot type; use a US-layout ASCII password")
        return 2
    seq.append((False, KEY_CODES["enter"]))
    script = """import os,fcntl,struct,time
seq=SEQ
fd=os.open('/dev/uinput',os.O_WRONLY|os.O_NONBLOCK)
def ev(code,value):
 os.write(fd,struct.pack('qqHHi',0,0,1,code,value))
 os.write(fd,struct.pack('qqHHi',0,0,0,0,0))
 time.sleep(.03)
try:
 fcntl.ioctl(fd,0x40045564,1)
 for c in set([x[1] for x in seq]+[42]):
  fcntl.ioctl(fd,0x40045565,c)
 os.write(fd,struct.pack('80sHHHHI',b'BifrOSt remote keyboard',3,1,1,1,0)+bytes(1024))
 fcntl.ioctl(fd,0x5501)
 time.sleep(.3)
 for shift,c in seq:
  if shift: ev(42,1)
  ev(c,1); ev(c,0)
  if shift: ev(42,0)
 fcntl.ioctl(fd,0x5502)
finally:
 os.close(fd)
""".replace("SEQ", repr(seq))
    result = run(SSH + ["python3 -c " + shlex.quote(script)], 20)
    if result.stderr:
        print(result.stderr.replace(password, "***"), end="", file=sys.stderr)
    return result.returncode


def cmd_login_bypass(args):
    """`login-bypass on|off`: root over ssh touches/removes /etc/bifrost/no-login. With it on, privd reports has_password:false
    and the login window accepts an empty password. Use it only to recover a broken login; turn it off again."""
    if args not in (["on"], ["off"]):
        print("usage: login-bypass on|off")
        return 2
    cmd = "install -d /etc/bifrost && touch /etc/bifrost/no-login" if args[0] == "on" else "rm -f /etc/bifrost/no-login"
    result = run(SSH + [cmd], 10)
    print(result.stderr, end="", file=sys.stderr)
    if result.returncode == 0:
        print("login-bypass " + args[0])
    return result.returncode


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


def bios_find(rows, word):
    """`NAME=current|possible` rows whose attribute name, value or choices contain WORD (case-insensitive substring)."""
    word = word.lower()
    return [r for r in rows if word in r.lower()]


def cmd_bios(args):
    """BIOS settings through the kernel's think-lmi driver (Lenovo WMI): read, search, change and save from the OS, no keyboard in the BIOS.
    A supervisor password, if one is set, goes in $BIOS_PASSWORD (never stored). Changes apply at the next boot."""
    op = args[0] if args else "list"
    if state() not in ("live", "installed"):
        print(f"midget is {state()}: bios needs ssh (boot it into the lab ISO first: `bifrost pc midget wake`, or netboot it by hand once)")
        return 1
    probe = f"modprobe think-lmi 2>/dev/null; [ -d {BIOS_BASE}/attributes ] || {{ echo NOLMI; ls /sys/bus/wmi/devices 2>/dev/null | head -20; dmidecode -s bios-version 2>/dev/null; exit 3; }}"
    if op in ("list", "dump", "find"):
        # attribute names hold spaces and parentheses ("Intel(R) Manageability Control"): every expansion is quoted
        out = ssh(probe + f"; cd {BIOS_BASE}/attributes; for a in *; do [ -f \"$a/current_value\" ] || continue; "
                  "echo \"$a=$(cat \"$a/current_value\")|$(cat \"$a/possible_values\" 2>/dev/null)\"; done", 60)
        if out.startswith("NOLMI"):
            print("think-lmi is not available on this kernel/BIOS (the WMI interface is missing). WMI devices seen:\n" + out[6:])
            print("Fallbacks: efibootmgr (`efi`, `bootnext`), Wake-on-LAN, AMT (`amt`), or the BIOS menu by hand (F1).")
            return 1
        rows = [l for l in out.splitlines() if "=" in l]
        if op == "find":
            rows = bios_find(rows, args[1] if len(args) > 1 else "")
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
        d = shlex.quote(f"{BIOS_BASE}/attributes/{args[1]}")  # exact attribute name; spaces and parentheses are fine
        print(ssh(probe + f"; [ -d {d} ] || {{ echo 'no such attribute (try: bios find WORD)'; exit 4; }}; "
                  f"echo \"current_value: $(cat {d}/current_value)\"; echo \"possible_values: $(cat {d}/possible_values 2>/dev/null)\""))
        return 0
    if op == "set" and len(args) > 2:
        name, value = args[1], args[2]
        d = shlex.quote(f"{BIOS_BASE}/attributes/{name}")
        pw = os.environ.get("BIOS_PASSWORD", "")
        auth = f"printf %s {shlex.quote(pw)} > {BIOS_BASE}/authentication/Admin/current_password; " if pw else ""
        out = ssh(probe + f"; [ -d {d} ] || {{ echo 'no such attribute (try: bios find WORD)'; exit 4; }}; "
                  f"echo \"possible_values: $(cat {d}/possible_values 2>/dev/null)\"; {auth}"
                  f"printf %s {shlex.quote(value)} > {d}/current_value && echo save > {BIOS_BASE}/attributes/save_settings && echo saved; "
                  f"echo \"current_value: $(cat {d}/current_value)\"", 40)
        print(out.replace(pw, "***") if pw else out)
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
        cmd_unlock()
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
        cmd_unlock()
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
        cmd_unlock()
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
    """The lab account's password (`lab_account`): out/lab/<username>.password (untracked, 0600), generated once when
    missing. It is published to the netbooted installer as out/lab/sync/lab/<username>.pw on titan's lab HTTP server
    (10.99.0.1:8078), never into the repo or the preset."""
    labdir = os.path.dirname(KEY)
    user = lab_account()[0]
    pw_file = os.path.join(labdir, f"{user}.password")
    if not os.path.exists(pw_file):
        import secrets
        fd = os.open(pw_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(secrets.token_urlsafe(12) + "\n")
    pw = open(pw_file).read().strip()
    pub = os.path.join(labdir, "sync", "lab")
    os.makedirs(pub, exist_ok=True)
    with open(os.path.join(pub, f"{user}.pw"), "w") as f:
        f.write(pw + "\n")
    return pw, pw_file


def ensure_lab_http():
    """The lab server must serve out/lab/sync: a server left running from a replaced directory answers 404 for every
    file, and bifrost-lab-sync then silently keeps the ISO's old installer. A stale one is restarted."""
    def served():
        try:
            return urllib.request.urlopen("http://10.99.0.1:8078/manifest", timeout=3).status == 200
        except Exception:
            return False
    if served():
        return True
    sync = os.path.join(os.path.dirname(KEY), "sync")
    if not os.path.exists(os.path.join(sync, "bifrost-installer")):
        return False
    subprocess.run(["pkill", "-f", "http.server 8078 --bind 10.99.0.1"])
    time.sleep(1)
    subprocess.Popen(["python3", "-m", "http.server", "8078", "--bind", "10.99.0.1"], cwd=sync, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    return wait(served, 10, 0.5)


MIRROR_URL = "http://10.99.0.1:8078/mirror/"


def write_preset(host, live_disk=None):
    """The unattended preset on Midget's ESP, as `bifrost-mcp lifecycle reinstall` writes it (mcp/src/lifecycle.rs), plus
    the lab's offline package mirror: written here so an installed system with an older MCP still gets the mirror."""
    if live_disk:
        disk_id = ssh(f"lsblk -dno WWN {shlex.quote(live_disk)}", 10).strip()
    else:
        disk_id = ssh('src=$(findmnt -no SOURCE /boot); pk=$(lsblk -no PKNAME "$src" | head -1); lsblk -dno WWN "/dev/$pk"', 10).strip()
    if not re.fullmatch(r"[A-Za-z0-9._:-]+", disk_id or ""):
        return None
    tz = "Atlantic/Reykjavik" if live_disk else ssh("readlink -f /etc/localtime | sed 's|.*/zoneinfo/||'", 5).strip()
    zone = tz if re.fullmatch(r"[A-Za-z0-9_+/-]+", tz or "") else "Atlantic/Reykjavik"
    now = int(time.time())
    user, full_name = lab_account()
    toml = (f"# written by tools/midget.py reinstall; read by bifrost-installer on the netbooted lab ISO\nversion = 1\n"
            f"disk_id = \"{disk_id}\"\nmac = \"00:23:24:dc:c8:40\"\nhostname = \"{host}\"\nlocale = \"en_US.UTF-8\"\nkeyboard = \"us\"\n"
            f"timezone = \"{zone}\"\nusername = \"{user}\"\nfull_name = \"{full_name}\"\nautologin = true\nmcp = true\n"
            f"password_url = \"http://10.99.0.1:8078/lab/{user}.pw\"\nmirror = \"{MIRROR_URL}\"\ncreated_epoch = {now}\n"
            f"expires_s = 1800\njob = \"{now}-reinstall\"\n")
    # a live system: the installer's own copy of the ESP stash in /run (it reads that first), for this disk
    dest = (f"cat > /run/bifrost-unattended.toml && printf %s {shlex.quote(live_disk)} > /run/bifrost-unattended.toml.disk" if live_disk
            else "mkdir -p /boot/EFI/BIFROST && cat > /boot/EFI/BIFROST/unattended.toml && sync")
    r = subprocess.run(SSH + [dest], input=toml, text=True, capture_output=True, timeout=20)
    return disk_id if r.returncode == 0 else None


def mcp_call(tool, args, timeout=30):
    """One bifrost-mcp tool call over ssh (the same binary the agents use; no token needed with the lab key)."""
    return ssh(f"bifrost-mcp call {tool} {shlex.quote(json.dumps(args))} 2>&1", timeout)


def cmd_reinstall(args):
    """`reinstall --confirm "REINSTALL midget" [--timeout S]`: MCP `lifecycle reinstall`, then follow the phases with timestamps."""
    confirm = args[args.index("--confirm") + 1] if "--confirm" in args and args.index("--confirm") + 1 < len(args) else ""
    budget = int(args[args.index("--timeout") + 1]) if "--timeout" in args else 2400
    every = shot_interval(args)
    st = state()
    if st not in ("installed", "live"):
        print(f"midget is {st}; reinstall starts from an installed or a live lab system (power it on, or `wake`)")
        return 1
    # a live lab ISO (a failed or interrupted install) installs as `bifrost`, the hostname the preset gives it
    host = "bifrost" if st == "live" else ssh("cat /proc/sys/kernel/hostname", 10).strip()
    if confirm != f"REINSTALL {host}":
        print(f'refused: pass --confirm "REINSTALL {host}"')
        return 2
    if not iventoy_unattended():
        print("iVentoy would stop at its boot menu (no default image or timeout 0): run tools/lab-up.sh first")
        return 1
    t0 = time.time()
    # what the live system pulls: the installer and shell under test, and every package pacstrap installs
    repo = Path(__file__).resolve().parent.parent
    for cmd in (["tools/lab-push.sh", "--skip-cargo" if "--skip-build" in args else "--installer-only"], ["tools/lab-mirror.sh"]):
        r = subprocess.run(cmd, cwd=repo, capture_output=True, text=True, timeout=1800)
        if r.returncode:
            print(f"{cmd[0]} failed: {(r.stderr or r.stdout)[-400:]}")
            return 1
    if not ensure_lab_http():
        print("lab HTTP server (10.99.0.1:8078) cannot serve out/lab/sync")
        return 1
    _, pw_file = lab_password()
    _stamp(f"phase 0/7 published installer, shell and package mirror ({time.time() - t0:.0f}s); password in {pw_file}")
    if st == "live":
        # no reboot: the preset goes where the installer looks first (/run, its copy of the ESP stash), for the disk the
        # live system sees, and the installer starts over
        disk = ssh("lsblk -dnpo NAME,TYPE,RM,SIZE -b | awk '$2==\"disk\" && $3==0 && $4>8000000000 && $1 !~ /loop|nbd|sr|zram/ {print $1}'", 10).split()
        if len(disk) != 1:
            _stamp(f"FAILED: expected one internal disk on the live system, found {disk}")
            return 1
        disk_id = write_preset(host, live_disk=disk[0])
        if not disk_id:
            _stamp("FAILED: could not write the unattended preset on the live system")
            return 1
        ssh("systemctl restart bifrost-installer", 30)
        _stamp(f"live system: preset for {disk[0]} ({disk_id}), packages from {MIRROR_URL}; installer restarted")
        return follow_install(t0, budget, live=True, every=every)
    disk_id = write_preset(host)
    if not disk_id:
        _stamp("FAILED: could not write the unattended preset on Midget's ESP")
        return 1
    efi = ssh("efibootmgr", 10)
    pxe = sorted([l for l in efi.splitlines() if l.startswith("Boot") and ("PXE" in l or "IPv4" in l or "IPV4" in l)],
                 key=lambda l: ("IPv4" not in l and "IPV4" not in l, l))
    if not pxe:
        _stamp("FAILED: no PXE boot entry on Midget")
        return 1
    ssh(f"efibootmgr -n {pxe[0][4:8]} >/dev/null; sync; nohup sh -c 'sleep 1; systemctl reboot' >/dev/null 2>&1 &", 15)
    _stamp(f"preset written (disk {disk_id}, packages from {MIRROR_URL}); BootNext={pxe[0][4:8]} (PXE), rebooting")
    return follow_install(t0, budget, live=False, every=every)


def shot_interval(args):
    """`--shots S`: seconds between screen grabs while following a reinstall (default 10)."""
    return float(args[args.index("--shots") + 1]) if "--shots" in args else 10.0


def cmd_reinstall_loop(args):
    """`reinstall ... --loop N`: N reinstalls back to back (the build is published once), then their times."""
    n = int(args[args.index("--loop") + 1]) if "--loop" in args else 1
    rest = [a for i, a in enumerate(args) if a != "--loop" and (i == 0 or args[i - 1] != "--loop")]
    results = []
    for i in range(n):
        t = time.time()
        rc = cmd_reinstall(rest + (["--skip-build"] if i > 0 and "--skip-build" not in rest else []))
        results.append((rc, time.time() - t))
        _stamp(f"run {i + 1}/{n}: {'OK' if rc == 0 else 'FAILED'} in {results[-1][1]:.0f}s")
        if rc:
            break
    if n > 1:
        print("\n".join(f"  run {i + 1}: {'OK' if rc == 0 else 'FAILED'} {secs:.0f}s" for i, (rc, secs) in enumerate(results)))
    return 0 if all(rc == 0 for rc, _ in results) else 1


def follow_install(t0, budget, live, every=10.0):
    """Phases 1-7 after the preset: netboot, the unattended install (the live journal streams to out/lab/logs), the new
    system's clean boot, the tester session and MCP. Screens are grabbed every `every` seconds into one folder per run,
    from the installer through first boot, the lock screen and the unlocked desktop."""
    def left():
        return budget - (time.time() - t0)
    if not live:
        if not wait(lambda: not ping(), 60, 1):
            _stamp("FAILED: midget never went down")
            return 1
        _stamp("phase 1/7 going down: rebooting")
        if not wait(lambda: state() == "live", min(600, left()), 3):
            _stamp("FAILED: no netboot into the live lab ISO (check iVentoy and BootNext with `efi`)")
            return 1
        _stamp("phase 2/7 netboot seen: live lab ISO is up")
    logs = Path(__file__).resolve().parent.parent / "out" / "lab" / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    run = time.strftime("%Y%m%d-%H%M%S")
    shots = logs / f"{run}-install"
    shots.mkdir(exist_ok=True)
    if state() == "live":
        # the live system's journal dies with it (its root is iVentoy's iSCSI disk): keep a copy on titan as it happens
        live_log = logs / f"{run}-live-journal.log"
        tail = subprocess.Popen(SSH + ["journalctl -f -b -o short-precise --no-pager"], stdout=open(live_log, "w"), stderr=subprocess.DEVNULL)
        ssh("touch /run/bifrost-shot-on", 10)
        _stamp(f"live journal streaming to {live_log}; screens every {every:g} s in {shots}/")
    else:
        live_log, tail = "(none: not live)", None
        _stamp(f"screens every {every:g} s in {shots}/")
    last_shot = [0.0]

    def install_shot(label="install"):
        """The current frame, at most every `every` s: the live installer's lab hook (/run/bifrost-screen.png), or
        the installed system's compositor or shell grab (as `screen`). Named HHMMSS-<label>.png."""
        if time.time() - last_shot[0] < every:
            return
        last_shot[0] = time.time()
        out = shots / time.strftime(f"%H%M%S-{label}.png")
        try:
            if state() == "live":
                r = subprocess.run(SSH + ["cat /run/bifrost-screen.png"], capture_output=True, timeout=20)
                if r.stdout[:4] == b"\x89PNG":
                    out.write_bytes(r.stdout)
            else:
                with contextlib.redirect_stdout(io.StringIO()):
                    cmd_screen(str(out))
        except (subprocess.SubprocessError, OSError, ImportError):
            pass

    def shooting(pred, label):
        """`pred` for `wait`, grabbing a frame on each poll."""
        def check():
            install_shot(label)
            return pred()
        return check
    seen, last_line = False, ""
    while left() > 0 and state() == "live":
        install_shot()
        j = ssh("journalctl -u bifrost-installer --no-pager -n 400 2>/dev/null | grep -E 'unattended|bifrost-installer: \\[' | tail -n 1", 15).strip()
        if j and not seen:
            seen = True
            _stamp("phase 3/7 installer running: " + j[-140:])
        elif j and j != last_line:
            print("    " + j[-150:], flush=True)
        last_line = j
        if "refused or failed" in j:
            _stamp("FAILED: " + j)
            return 1
        time.sleep(min(5, every))
    if tail:
        tail.terminate()
    if state() != "installed" and not wait(shooting(lambda: state() == "installed", "reboot"), left(), min(3, every)):
        _stamp(f"FAILED: the installed system did not answer on ssh in time (live journal: {live_log})")
        return 1
    if not seen:
        _stamp("note: never saw installer progress lines; the install may have run between polls")
    _stamp("phase 4/7 installed system up (ssh with the lab key)")
    # a fresh install has no update in flight: the guard reports an empty phase; only an update leaves one to accept
    def booted_ok():
        g = ssh("bifrost-update-guard status", 15) or ""
        return ("phase=accepted" in g or "phase=\n" in g + "\n") and "ready=1" in (control_text("state") or "")
    if not wait(shooting(booted_ok, "boot"), min(300, left()), min(5, every)):
        _stamp("FAILED: the new system did not boot cleanly (update guard phase, shell ready)")
        return 1
    _stamp("phase 5/7 phase=accepted")
    account = lab_account()[0]
    def tester_up():
        return bool(ssh(f"getent passwd {account} >/dev/null && ps -u {account} -o comm= | head -n 1", 15).strip())
    def locked():
        return shell_screen() == "lock"
    ok = wait(shooting(tester_up, "session"), min(180, left()), min(5, every))
    if ok and locked():
        last_shot[0] = 0.0
        install_shot("lock")
        cmd_unlock()
        wait(shooting(lambda: not locked(), "unlock"), 20, min(1, every))
        last_shot[0] = 0.0
        install_shot("desktop")
    _stamp(f"phase 6/7 {account} session " + ("running" if ok else "NOT running"))
    mcp = wait(lambda: port_open(7420) and "host=" in mcp_call("system", {"what": "overview"}, 20), min(120, left()), 5)
    _stamp("phase 7/7 MCP " + ("reachable on :7420 and answering" if mcp else "NOT reachable"))
    _stamp(f"done in {time.time() - t0:.0f}s: " + ("OK" if ok and mcp else "INCOMPLETE"))
    return 0 if ok and mcp else 1


# ---- qa-install: wipe Midget through the installer's real screens, like a QA tester ----

STATE_FILE = "/run/bifrost-installer/state"


def parse_installer_state(text):
    """The installer's state file -> {size, screen, stage, percent, buttons{id: rect}, fields{id: (rect, focused)}}.
    Rects are (x, y, w, h) in panel pixels."""
    st = {"size": None, "screen": "", "stage": "", "percent": 0, "buttons": {}, "fields": {}}
    for line in text.splitlines():
        key, _, val = line.partition("=")
        parts = val.split()
        try:
            if key == "size" and len(parts) == 2:
                st["size"] = (int(parts[0]), int(parts[1]))
            elif key == "screen":
                st["screen"] = val.strip()
            elif key == "stage":
                st["stage"] = val.strip()
            elif key == "percent":
                st["percent"] = int(val)
            elif key == "button" and len(parts) == 5:
                st["buttons"][parts[0]] = tuple(int(v) for v in parts[1:5])
            elif key == "field" and len(parts) == 6:
                st["fields"][parts[0]] = (tuple(int(v) for v in parts[1:5]), parts[5] == "1")
        except ValueError:
            continue
    return st


def rect_centre(rect):
    x, y, w, h = rect
    return x + w // 2, y + h // 2


def find_button(st, *ids, prefix=None):
    """The centre of the first listed button id on screen, or of the first button starting with `prefix`; None when absent."""
    for i in ids:
        if i in st["buttons"]:
            return rect_centre(st["buttons"][i])
    if prefix:
        for i in sorted(st["buttons"]):
            if i.startswith(prefix):
                return rect_centre(st["buttons"][i])
    return None


def abs_pointer(x, y, size, top=65535):
    """Panel pixel (x, y) on a panel of `size` (w, h) -> uinput absolute axis values 0..top."""
    w, h = size
    cl = lambda v, hi: max(0, min(hi, v))
    return (round(cl(x, w - 1) * top / max(1, w - 1)), round(cl(y, h - 1) * top / max(1, h - 1)))


_UINPUT_PRELUDE = """import os,fcntl,struct,time
fd=os.open('/dev/uinput',os.O_WRONLY|os.O_NONBLOCK)
def ev(t,code,value):
 os.write(fd,struct.pack('qqHHi',0,0,t,code,value))
def syn():
 ev(0,0,0); time.sleep(.03)
"""


def uinput_pointer_script(points):
    """Python source run on Midget: a temporary absolute pointer that clicks each (ax, ay)."""
    return _UINPUT_PRELUDE + f"""pts={points!r}
try:
 fcntl.ioctl(fd,0x40045564,1); fcntl.ioctl(fd,0x40045564,3)
 fcntl.ioctl(fd,0x40045565,0x110)
 fcntl.ioctl(fd,0x40045567,0); fcntl.ioctl(fd,0x40045567,1)
 absmax=[65535,65535]+[0]*62
 os.write(fd,struct.pack('80sHHHHI',b'BifrOSt remote pointer',3,1,2,1,0)+struct.pack('64i',*absmax)+bytes(768))
 fcntl.ioctl(fd,0x5501)
 time.sleep(.5)
 for x,y in pts:
  ev(3,0,x); ev(3,1,y); syn(); time.sleep(.1)
  ev(1,0x110,1); syn(); time.sleep(.08)
  ev(1,0x110,0); syn(); time.sleep(.2)
 fcntl.ioctl(fd,0x5502)
finally:
 os.close(fd)
"""


def uinput_keys_script(seq):
    """Python source run on Midget: a temporary keyboard typing [(shift, code)]."""
    return _UINPUT_PRELUDE + f"""seq={seq!r}
try:
 fcntl.ioctl(fd,0x40045564,1)
 for c in set([x[1] for x in seq]+[42]):
  fcntl.ioctl(fd,0x40045565,c)
 os.write(fd,struct.pack('80sHHHHI',b'BifrOSt remote keyboard',3,1,1,1,0)+bytes(1024))
 fcntl.ioctl(fd,0x5501)
 time.sleep(.5)
 for shift,c in seq:
  if shift: ev(1,42,1); syn()
  ev(1,c,1); syn(); ev(1,c,0); syn()
  if shift: ev(1,42,0); syn()
 fcntl.ioctl(fd,0x5502)
finally:
 os.close(fd)
"""


def qa_tap(st, centre):
    ax, ay = abs_pointer(centre[0], centre[1], st["size"])
    return run(SSH + ["python3 -c " + shlex.quote(uinput_pointer_script([(ax, ay)]))], 30).returncode == 0


def fifo_key_lines(seq):
    """[(shift, code[, altgr])] -> the installer input FIFO's `key CODE 1|0` lines (Shift = 42 and AltGr = 100 held
    around the keys that need them)."""
    out = []
    for shift, code, *rest in seq:
        mods = ([42] if shift else []) + ([100] if rest and rest[0] else [])
        out += [f"key {m} 1" for m in mods] + [f"key {code} 1", f"key {code} 0"] + [f"key {m} 0" for m in reversed(mods)]
    return out


def qa_keys(lines):
    """Send `key CODE 1|0` lines to the installer's input FIFO, one event per write, paced, so its frame loop sees each."""
    payload = "\n".join(lines) + "\n"
    return run(SSH + ["python3 -c " + shlex.quote(
        "import sys,time\n"
        "lines=sys.stdin.read().splitlines()\n"
        "with open('/run/bifrost-input','w') as f:\n"
        "    for l in lines:\n"
        "        f.write(l+'\\n'); f.flush(); time.sleep(0.03)\n")], 60, input=payload)


def qa_type(text, secret=False, layout="us"):
    """Type into the focused installer field as key presses on `layout` through its input FIFO (/run/bifrost-input), the
    path the installer always reads. A fresh uinput keyboard is not picked up in time (2026-10-12: every field stayed empty)."""
    seq = layout_keys(text, layout)
    if seq is None:
        return False
    # clear what the field already holds first (the installer pre-fills the account name from the full name), the way a
    # user would: Backspace until empty (64 is more than any field here holds)
    r = qa_keys(["key 14 1", "key 14 0"] * 64 + fifo_key_lines(seq))
    if r.stderr:
        print(r.stderr.replace(text, "***") if secret else r.stderr, end="", file=sys.stderr)
    return r.returncode == 0


def qa_state():
    r = run(SSH + [f"cat {STATE_FILE} 2>/dev/null"], 15)
    return parse_installer_state(r.stdout) if r.returncode == 0 and r.stdout.strip() else None


def internal_disks():
    return ssh("lsblk -dnpo NAME,TYPE,RM | awk '$2==\"disk\" && $3==0 {print $1}'", 10).split()


def cmd_qa_install(args):
    """`qa-install [--disk DEV] [--shots DIR] [--timeout S]`: wipe and reinstall Midget by driving the installer's real
    screens with taps and typing (no unattended preset), then follow the first boot. Report: out/lab/logs/<stamp>-qa-install/report.md."""
    opt = lambda n, d=None: args[args.index(n) + 1] if n in args and args.index(n) + 1 < len(args) else d
    want_disk = opt("--disk")
    budget = int(opt("--timeout", "2400"))
    stamp = time.strftime("%Y%m%d-%H%M%S")
    logdir = Path(REPO) / "out" / "lab" / "logs" / f"{stamp}-qa-install"
    shots = Path(opt("--shots", str(logdir / "shots")))
    shots.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    rows = []

    def step(msg, shot=None):
        _stamp(msg)
        rows.append((time.strftime("%H:%M:%S"), time.time() - t0, msg, shot))

    def report(result):
        lines = [f"# qa-install {stamp}: {result}", "", "| time | +s | step | screenshot |", "|---|---|---|---|"]
        for t, dt, msg, shot in rows:
            lines.append(f"| {t} | {dt:.0f} | {msg} | {shot or ''} |")
        (logdir / "report.md").write_text("\n".join(lines) + "\n")

    def snap(label):
        out = shots / time.strftime(f"%H%M%S-{label}.png")
        try:
            r = subprocess.run(SSH + ["cat /run/bifrost-screen.png"], capture_output=True, timeout=20)
            if r.stdout[:4] == b"\x89PNG":
                out.write_bytes(r.stdout)
                return str(out)
        except (subprocess.SubprocessError, OSError):
            pass
        return None

    def fail(msg):
        shot = snap("failure")
        step(f"FAILED: {msg}" + (f" (last screenshot {shot})" if shot else ""), shot)
        report("FAILED")
        return 1

    def screen(name, bound):
        """Wait for the installer to publish screen `name`; its state, or None."""
        box = {}
        def seen():
            s = qa_state()
            if s and s["screen"] == name and s["size"]:
                box["s"] = s
                return True
            return False
        ssh("touch /run/bifrost-shot-on", 10)
        return box["s"] if wait(seen, bound, 1.5) else None

    st = state()
    if st not in ("installed", "live"):
        print(f"midget is {st}; qa-install starts from an installed or a live lab system")
        return 1
    password = login_password()
    user, full_name = lab_account()
    layout = lab_keyboard()
    tables = installer_keymaps()
    if layout not in tables[0]:
        print(f"keyboard={layout} in out/lab/account.conf is not an installer layout ({', '.join(tables[0])})")
        return 2
    if password is None or layout_keys(password, layout, tables) is None:
        print(f"no lab password typeable on the {layout} layout: write out/lab/<username>.password")
        return 2
    if layout_keys(full_name, layout, tables) is None:
        print(f"the full name has a character the {layout} layout cannot type: set keyboard= in out/lab/account.conf (is, us-intl, ...)")
        return 2
    if st == "installed":
        ssh("rm -f /boot/EFI/BIFROST/unattended.toml; sync", 15)
        efi = ssh("efibootmgr", 10)
        pxe = sorted([l for l in efi.splitlines() if l.startswith("Boot") and ("PXE" in l or "IPv4" in l or "IPV4" in l)],
                     key=lambda l: ("IPv4" not in l and "IPV4" not in l, l))
        if not pxe:
            return fail("no PXE boot entry on Midget")
        ssh(f"efibootmgr -n {pxe[0][4:8]} >/dev/null; sync; nohup sh -c 'sleep 1; systemctl reboot' >/dev/null 2>&1 &", 15)
        step(f"stale preset removed; BootNext={pxe[0][4:8]} (PXE), rebooting")
        if not wait(lambda: not ping(), 60, 1):
            return fail("midget never went down")
        if not wait(lambda: state() == "live", 600, 3):
            return fail("no netboot into the live lab ISO")
    else:
        ssh("rm -f /run/bifrost-unattended.toml /run/bifrost-unattended.toml.disk; systemctl restart bifrost-installer", 30)
    step("live lab ISO is up")
    s = screen("welcome", 300)
    if not s:
        return fail("the installer state file never showed the Welcome screen")
    step("Welcome screen", snap("welcome"))
    if not qa_tap(s, find_button(s, "next") or (0, 0)):
        return fail("no Continue button on Welcome")
    s = screen("language", 60)
    if not s:
        return fail("the Language screen did not appear")
    step("Language screen", snap("language"))
    en = find_button(s, "lang0")
    if en:
        qa_tap(s, en)
        s = qa_state() or s
    # the keyboard column, the way a keyboard user picks it: Tab to the column, Up to the top, Down to the layout
    li = tables[0].index(layout)
    qa_keys(["key 15 1", "key 15 0"] + ["key 103 1", "key 103 0"] * len(tables[0]) + ["key 108 1", "key 108 0"] * li)
    time.sleep(0.5)
    s = qa_state() or s
    step(f"keyboard layout {layout}", snap("language-keyboard"))
    if not qa_tap(s, find_button(s, "next") or (0, 0)):
        return fail("no Continue button on Language")
    s = screen("account", 60)
    if not s:
        return fail("the Account screen did not appear")
    step("Account screen", snap("account"))
    for fid, text, secret in (("name", full_name, False), ("username", user, False), ("password", password, True), ("verify", password, True)):
        if fid not in s["fields"]:
            return fail(f"no {fid} field on Account")
        qa_tap(s, rect_centre(s["fields"][fid][0]))
        time.sleep(0.4)
        if not qa_type(text, secret, layout):
            return fail(f"could not type the {fid} field")
        s = qa_state() or s
    step("account typed (password not shown)", snap("account-filled"))
    c = find_button(s, "next")
    if not c or not qa_tap(s, c):
        return fail("Continue is not available on Account (fields rejected?)")
    s = screen("look", 8) or screen("disk", 60)
    if s and s["screen"] == "look":
        step("Look screen", snap("look"))
        qa_tap(s, find_button(s, "next") or (0, 0))
        s = screen("disk", 60)
    if not s:
        return fail("the Disk screen did not appear")
    step("Disk screen", snap("disk"))
    disks = sorted((i for i in s["buttons"] if re.fullmatch(r"disk\d+", i)), key=lambda i: int(i[4:]))
    names = internal_disks()
    target = want_disk
    if not target:
        nvme = [n for n in names if "nvme" in n]
        if len(nvme) != 1:
            return fail(f"expected one internal NVMe disk, found {nvme}; pass --disk")
        target = nvme[0]
    if target not in names:
        return fail(f"disk {target} is not an internal disk on Midget ({names})")
    pick = disks[names.index(target)] if len(disks) == len(names) else (disks[0] if len(disks) == 1 else None)
    if pick is None:
        return fail(f"cannot match {target} to the Disk-screen buttons {disks}")
    qa_tap(s, rect_centre(s["buttons"][pick]))
    time.sleep(0.5)
    s = qa_state() or s
    step(f"picked {pick} = {target}", snap("disk-picked"))
    if not qa_tap(s, find_button(s, "install", "next") or (0, 0)):
        return fail("no Install button on Disk")
    s = screen("confirm", 30)
    if not s:
        return fail("the Confirm screen did not appear")
    step("Confirm screen", snap("confirm"))
    c = find_button(s, "erase", "install", "next")
    if not c or not qa_tap(s, c):
        return fail("no Erase/Install button on Confirm")
    if not screen("progress", 60):
        return fail("the Progress screen did not appear")
    step("Progress screen", snap("progress"))
    last, deadline = "", time.time() + min(budget, 1800)
    s = None
    while time.time() < deadline:
        s = qa_state()
        if s and s["screen"] in ("done", "failed"):
            break
        if s:
            line = f"{s['stage']} {s['percent']}%"
            if line != last:
                print("    " + line, flush=True)
                last = line
        time.sleep(5)
    if not s or s["screen"] != "done":
        return fail(f"install did not reach Done (screen={s['screen'] if s else 'none'}, {last})")
    step("Done screen", snap("done"))
    # a reboot shows as a new kernel boot id (Midget can be down for less than a ping interval, and the installer first
    # syncs and unmounts, so a ping gap is not a reliable sign)
    live_boot = ssh("cat /proc/sys/kernel/random/boot_id", 10).strip()
    rebooted = lambda: (b := ssh("cat /proc/sys/kernel/random/boot_id", 5).strip()) != "" and b != live_boot
    c = find_button(s, "restart")
    if c and qa_tap(s, c) and wait(lambda: not ping() or rebooted(), 90, 2):
        step("tapped Restart")
    else:
        # Restart is the default button: Return presses it
        qa_keys(["key 28 1", "key 28 0"])
        if not wait(lambda: not ping() or rebooted(), 90, 2):
            return fail("Restart (tap and Return) did not reboot")
        step("pressed Return on Restart")
    rc = qa_first_boot(step)
    report("OK" if rc == 0 else "FAILED (first boot)")
    return rc


def qa_first_boot(step):
    """Phases 4-7 of `reinstall`: installed system up, phase=accepted, the account's session, MCP."""
    if not wait(lambda: not ping() or state() == "installed", 120, 2):
        step("FAILED: midget neither went down nor came up after Restart")
        return 1
    if not wait(lambda: state() == "installed", 600, 3):
        step("FAILED: the installed system did not answer on ssh")
        return 1
    step("installed system up")

    def booted_ok():
        g = ssh("bifrost-update-guard status", 15) or ""
        return ("phase=accepted" in g or "phase=\n" in g + "\n") and "ready=1" in (control_text("state") or "")
    ok = wait(booted_ok, 300, 5)
    step("phase=accepted" if ok else "FAILED: the new system did not boot cleanly")
    account = lab_account()[0]
    up = wait(lambda: bool(ssh(f"getent passwd {account} >/dev/null && ps -u {account} -o comm= | head -n 1", 15).strip()), 180, 5)
    if up and shell_screen() == "lock":
        cmd_unlock()
    step(f"{account} session " + ("running" if up else "NOT running"))
    mcp = wait(lambda: port_open(7420) and "host=" in mcp_call("system", {"what": "overview"}, 20), 120, 5)
    step("MCP " + ("reachable on :7420" if mcp else "NOT reachable"))
    return 0 if ok and up and mcp else 1


KBENCH_PACKAGES = "rt-tests stress-ng numactl"  # cyclictest needs libnuma (numactl)
PERF_HISTORY = os.path.join(REPO, "docs/perf-history.jsonl")


def parse_cyclic(text):
    """cyclictest output -> {"min": [us per CPU], "avg": [...], "max": [...]} from its `# Min/Avg/Max Latencies:` lines."""
    out = {}
    for key in ("Min", "Avg", "Max"):
        m = re.search(rf"^# {key} Latencies:\s*(.+)$", text, re.M)
        out[key.lower()] = [int(v) for v in m.group(1).split()] if m else []
    return out


def count_underruns(text):
    """aplay prints `underrun!!!` for every xrun."""
    return sum(1 for l in text.splitlines() if "underrun" in l)


def kbench_result(label, files, seconds=60):
    """The record `kbench` appends to docs/perf-history.jsonl from the raw files of /root/kbench/LABEL
    (keys: kernel, cyclic-idle, cyclic-stress, aplay-32/64/128). Underruns are per minute: scaled when SECONDS is not 60."""
    kernel = (files.get("kernel", "").splitlines() or [""])[0].split()
    idle, stress = parse_cyclic(files.get("cyclic-idle", "")), parse_cyclic(files.get("cyclic-stress", ""))
    scale = 60.0 / seconds if seconds else 1.0
    return {"date": time.strftime("%Y-%m-%d %H:%M"), "bench": "kernel-latency", "label": label,
            "kernel": kernel[2] if len(kernel) > 2 else "",
            "max_us_idle": max(idle["max"], default=None), "max_us_stress": max(stress["max"], default=None),
            "underruns": {p: round(count_underruns(files.get("aplay-" + p, "")) * scale) for p in ("32", "64", "128")}}


def kbench_summary(rec, files):
    """Printable summary: kernel, cyclictest Min/Avg/Max per CPU idle and stressed, aplay underruns per period."""
    lines = [f"kernel {rec['kernel']} ({rec['label']})"]
    for name, key in (("idle", "cyclic-idle"), ("stress", "cyclic-stress")):
        c = parse_cyclic(files.get(key, ""))
        lines.append(f"cyclictest {name} (us per CPU)")
        for k in ("min", "avg", "max"):
            lines.append(f"  {k.capitalize():3} " + " ".join(f"{v:4}" for v in c[k]))
    lines.append("aplay underruns per minute: " + "  ".join(f"{p} frames={n}" for p, n in rec["underruns"].items()))
    return "\n".join(lines)


def kbench_fetch(label):
    """Raw result files of /root/kbench/LABEL as {name: text}."""
    names = "kernel.txt cyclic-idle.txt cyclic-stress.txt aplay-32.txt aplay-64.txt aplay-128.txt"
    out = ssh(f"cd /root/kbench/{shlex.quote(label)} && for f in {names}; do echo \"=====$f\"; cat $f; done", 60)
    files, cur = {}, None
    for line in out.splitlines(keepends=True):
        if line.startswith("====="):
            cur = line[5:].strip().removesuffix(".txt")
            files[cur] = ""
        elif cur:
            files[cur] += line
    return files


def seconds_arg(args, default=60):
    return int(args[args.index("--seconds") + 1]) if "--seconds" in args else default


def kbench_start(label, seconds):
    """Install tools/midget/kbench.sh, make sure the packages are there and start it detached. Returns False on failure."""
    script = Path(REPO, "tools/midget/kbench.sh").read_bytes()
    r = subprocess.run(SSH + ["cat > /root/kbench.sh && chmod +x /root/kbench.sh"], input=script, capture_output=True, timeout=30)
    if r.returncode:
        print("could not install /root/kbench.sh")
        return False
    pac = run(SSH + [f"pacman -S --needed --noconfirm {KBENCH_PACKAGES} 2>&1 | tail -3"], 300)
    print(pac.stdout, end="")
    ssh(f"rm -rf /root/kbench/{shlex.quote(label)}; nohup /root/kbench.sh {shlex.quote(label)} {seconds} > /root/kbench-{shlex.quote(label)}.log 2>&1 &", 15)
    return True


def kbench_wait(label, seconds):
    """Wait for /root/kbench/LABEL/DONE (cyclictest twice + aplay three times + margin)."""
    return wait(lambda: ssh(f"test -f /root/kbench/{shlex.quote(label)}/DONE && echo yes", 15).strip() == "yes", seconds * 6 + 180, 10)


def kbench_run(label, seconds):
    """Run one bench, print its summary and append a JSON line to docs/perf-history.jsonl. Returns the record or None."""
    if state() != "installed":
        print(f"midget is {state()}: kbench needs an installed system over ssh")
        return None
    print(f"kbench {label}: about {seconds * 5 // 60 + 1} min")
    if not kbench_start(label, seconds) or not kbench_wait(label, seconds):
        print("kbench did not finish")
        return None
    files = kbench_fetch(label)
    rec = kbench_result(label, files, seconds)
    print(kbench_summary(rec, files))
    with open(PERF_HISTORY, "a") as f:
        f.write(json.dumps(rec) + "\n")
    return rec


def cmd_kbench(args):
    """`kbench LABEL [--seconds N]`: kernel latency bench on the running kernel (default 60 s per step)."""
    if not args or args[0].startswith("--"):
        print("usage: kbench LABEL [--seconds N]")
        return 2
    return 0 if kbench_run(args[0], seconds_arg(args)) else 1


RT_ENTRY = "BifrOSt RT bench"
# efibootmgr -C (create only): the entry is not put into BootOrder; BootNext boots it once. The kernel line is the
# current one with its initrd swapped for the RT image, and panic=10 so a failed boot falls back to the disk entry.
RT_ENTRY_SCRIPT = r"""
n=$(efibootmgr | sed -n 's/^Boot\([0-9A-Fa-f]\{4\}\)\*\? BifrOSt RT bench.*/\1/p' | head -1)
if [ -z "$n" ]; then
  esp=$(findmnt -no SOURCE /boot/efi || findmnt -no SOURCE /efi || findmnt -no SOURCE /boot)
  b=$(basename "$esp"); disk=/dev/$(lsblk -no PKNAME "$esp"); part=$(cat /sys/class/block/$b/partition)
  args="$(sed 's/initrd=[^ ]*//g' /proc/cmdline) initrd=\\initramfs-linux-rt.img panic=10"
  efibootmgr -C -d "$disk" -p "$part" -L 'BifrOSt RT bench' -l '\vmlinuz-linux-rt' -u "$args" >/dev/null
  n=$(efibootmgr | sed -n 's/^Boot\([0-9A-Fa-f]\{4\}\)\*\? BifrOSt RT bench.*/\1/p' | head -1)
fi
[ -n "$n" ] && efibootmgr -n "$n" >/dev/null && echo "$n"
"""


def cmd_kab(args):
    """`kab [--seconds N]`: kernel A/B. kbench on the running kernel (stock before 2026-10-12; linux-rt is the installed default now, so boot stock `linux` for a real A/B), one boot into linux-rt through a create-only
    EFI entry plus BootNext, kbench there, both summaries side by side. Stops when the RT kernel did not come up
    (the entry sets panic=10, so a failed boot returns to the disk entry)."""
    seconds = seconds_arg(args)
    stock = kbench_run("stock", seconds)
    if not stock:
        return 1
    num = ssh(RT_ENTRY_SCRIPT, 30).strip().splitlines()
    if not num or len(num[-1]) != 4:
        print("could not create or select the 'BifrOSt RT bench' EFI entry (is linux-rt installed? pacman -S linux-rt)")
        return 1
    print(f"BootNext={num[-1]}; rebooting into linux-rt")
    schedule_power("reboot")
    wait(lambda: not ping(), 60, 1)
    if not wait(lambda: state() == "installed", 300, 5):
        print("did not come back in 5 min")
        return 1
    kernel = ssh("uname -r", 15).strip()
    if "rt" not in kernel:
        print(f"booted {kernel!r}, not linux-rt (panic fallback or BootNext ignored); stopping")
        return 2
    time.sleep(20)  # services settle after boot
    rt = kbench_run("rt", seconds)
    if not rt:
        return 1
    print("\n" + "-" * 60)
    sf, rf = kbench_fetch("stock"), kbench_fetch("rt")
    a, b = kbench_summary(stock, sf).splitlines(), kbench_summary(rt, rf).splitlines()
    width = max(len(l) for l in a) + 3
    for i in range(max(len(a), len(b))):
        print((a[i] if i < len(a) else "").ljust(width) + (b[i] if i < len(b) else ""))
    return 0


AMT_PASSWORD_FILE = os.path.join(os.path.dirname(KEY), "amt.password")
AMT_ADDRESS = "10.99.0.201"
RPC_VERSION = "v2.52.7"
RPC_URL = f"https://github.com/device-management-toolkit/rpc-go/releases/download/{RPC_VERSION}/rpc_linux_x64.tar.gz"
RPC_BIN = "/root/rpc/rpc_linux_x64"


def amt_password(create=False):
    """The AMT admin password from out/lab/amt.password (untracked, mode 0600). With `create`, a missing file is generated:
    AMT wants 8 to 32 characters with upper, lower, digit and a special one. Never printed."""
    path = AMT_PASSWORD_FILE
    if not os.path.exists(path):
        if not create:
            return None
        import secrets, string
        pw = "Bf" + "".join(secrets.choice(string.ascii_letters + string.digits) for _ in range(20)) + "7!"
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(pw + "\n")
    st = os.stat(path)
    if st.st_mode & 0o077:
        raise SystemExit(f"{path} must be mode 600 (chmod 600 {path}); refusing to read it")
    return open(path).read().strip() or None


def redact(text, secret):
    return text.replace(secret, "<amt-password>") if secret else text


def rpc_install():
    """Intel's rpc-go (pinned release) in /root/rpc on midget, fetched on the machine when missing."""
    r = run(SSH + [f"[ -x {RPC_BIN} ] || {{ mkdir -p /root/rpc && cd /root/rpc && curl -fsSL {RPC_URL} | tar xz; }}; [ -x {RPC_BIN} ] && echo ok"], 120)
    return r.stdout.strip().endswith("ok")


def cmd_amt_info():
    """`amt-info`: rpc-go `amtinfo` on midget (ME firmware, AMT version, control mode, provisioning state, LAN)."""
    if state() not in ("live", "installed"):
        print(f"midget is {state()}: needs ssh")
        return 1
    if not rpc_install():
        print(f"could not fetch rpc-go {RPC_VERSION} on midget (no internet from the lab?): {RPC_URL}")
        return 1
    print(ssh(f"{RPC_BIN} amtinfo 2>&1", 60))
    return 0


def cmd_amt_activate():
    """`amt-activate`: client control mode (CCM) activation with rpc-go, then the static AMT address 10.99.0.201/24 gw 10.99.0.1.
    The password comes from out/lab/amt.password (created 0600 when missing) and is redacted from all output.
    Known on midget (2026-10-12): activation fails with EBUSY on the LME client (the MEI LME client allows one connection,
    max_conn 1) with rpc-go v2.52.7 and v2.39.0 on both kernels. Next: Intel LMS (the Local Manageability Service holds that
    connection), then an ME firmware update, only with the owner's go-ahead (a failed flash can brick an M900 Tiny)."""
    if state() not in ("live", "installed"):
        print(f"midget is {state()}: needs ssh")
        return 1
    pw = amt_password(create=True)
    if not rpc_install():
        print(f"could not fetch rpc-go {RPC_VERSION} on midget: {RPC_URL}")
        return 1
    q = shlex.quote(pw)
    steps = [("activate (CCM)", f"{RPC_BIN} activate -local -ccm -password {q} 2>&1 | tail -5"),
             ("static address on the shared wired port",
              f"{RPC_BIN} configure wired -static -ipaddress {AMT_ADDRESS} -subnetmask 255.255.255.0 -gateway 10.99.0.1 -primarydns 10.99.0.1 -password {q} 2>&1 | tail -5"),
             ("state", f"{RPC_BIN} amtinfo 2>&1 | grep -E 'Control Mode|AMT IP Address|Link Status' | head -4")]
    code = 0
    for title, cmd in steps:
        print(f"== {title}")
        r = run(SSH + [cmd], 180)
        out = redact(r.stdout + r.stderr, pw)
        print(out, end="" if out.endswith("\n") else "\n")
        if title.startswith("activate") and ("EBUSY" in out or "busy" in out.lower()):
            print("activation blocked: the LME client is busy (single connection). See `amt-activate` in the docstring: try Intel LMS next.")
            code = 1
            break
    return code


WSMAN_NS = {"cim": "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/", "a": "http://schemas.xmlsoap.org/ws/2004/08/addressing",
            "w": "http://schemas.dmtf.org/wbem/wsman/1/wsman.xsd", "s": "http://www.w3.org/2003/05/soap-envelope"}
POWER_STATES = {"on": 2, "off": 8, "cycle": 5, "reset": 10}  # CIM_PowerManagementService PowerState values (Intel AMT)
POWER_NAMES = {2: "on", 3: "sleep (light)", 4: "sleep (deep)", 7: "hibernate", 8: "off (soft)", 12: "off (hard)"}


def wsman_envelope(address, resource, action, body, selectors=""):
    """A WS-Management SOAP request to http://ADDRESS:16992/wsman for `action` on `resource` (a CIM class name)."""
    return (f'<?xml version="1.0" encoding="UTF-8"?>'
            f'<s:Envelope xmlns:s="{WSMAN_NS["s"]}" xmlns:a="{WSMAN_NS["a"]}" xmlns:w="{WSMAN_NS["w"]}"><s:Header>'
            f'<a:To>http://{address}:16992/wsman</a:To>'
            f'<w:ResourceURI s:mustUnderstand="true">{WSMAN_NS["cim"]}{resource}</w:ResourceURI>'
            f'<a:ReplyTo><a:Address s:mustUnderstand="true">http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</a:Address></a:ReplyTo>'
            f'<a:Action s:mustUnderstand="true">{action}</a:Action>'
            f'<a:MessageID>uuid:00000000-0000-0000-0000-000000000001</a:MessageID>'
            f'{selectors}<w:OperationTimeout>PT30S</w:OperationTimeout></s:Header><s:Body>{body}</s:Body></s:Envelope>')


def power_request_xml(address, state_name):
    """RequestPowerStateChange on CIM_PowerManagementService: PowerState 2 on, 8 off, 5 cycle, 10 reset."""
    code = POWER_STATES[state_name]
    cim = WSMAN_NS["cim"]
    body = (f'<p:RequestPowerStateChange_INPUT xmlns:p="{cim}CIM_PowerManagementService"><p:PowerState>{code}</p:PowerState>'
            f'<p:ManagedElement><a:Address>http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</a:Address>'
            f'<a:ReferenceParameters><w:ResourceURI>{cim}CIM_ComputerSystem</w:ResourceURI><w:SelectorSet>'
            f'<w:Selector Name="CreationClassName">CIM_ComputerSystem</w:Selector><w:Selector Name="Name">ManagedSystem</w:Selector>'
            f'</w:SelectorSet></a:ReferenceParameters></p:ManagedElement></p:RequestPowerStateChange_INPUT>')
    return wsman_envelope(address, "CIM_PowerManagementService",
                          f"{cim}CIM_PowerManagementService/RequestPowerStateChange", body)


def power_state_xml(address):
    """Get CIM_AssociatedPowerManagementService, whose PowerState is the current state."""
    cim = WSMAN_NS["cim"]
    return wsman_envelope(address, "CIM_AssociatedPowerManagementService",
                          "http://schemas.xmlsoap.org/ws/2004/09/transfer/Get", "")


def parse_power_response(xml_text):
    """-> {"power_state": int|None, "return_value": int|None, "fault": str|None} from a WS-Management reply (namespaces ignored)."""
    import xml.etree.ElementTree as ET
    out = {"power_state": None, "return_value": None, "fault": None}
    try:
        root = ET.fromstring(xml_text)
    except ET.ParseError:
        out["fault"] = "reply is not XML"
        return out
    for el in root.iter():
        tag = el.tag.rsplit("}", 1)[-1]
        text = (el.text or "").strip()
        if tag == "PowerState" and text.lstrip("-").isdigit() and out["power_state"] is None:
            out["power_state"] = int(text)
        elif tag == "ReturnValue" and text.isdigit():
            out["return_value"] = int(text)
        elif tag in ("Text", "faultstring") and text and out["fault"] is None:
            out["fault"] = text
    return out


def power_describe(parsed, action=None):
    """One line for the user from a parsed reply."""
    if parsed["fault"]:
        return "AMT fault: " + parsed["fault"]
    if action:
        return f"power {action}: " + ("accepted" if parsed["return_value"] == 0 else f"ReturnValue {parsed['return_value']}")
    n = parsed["power_state"]
    return "power state: " + (f"{POWER_NAMES.get(n, 'unknown')} ({n})" if n is not None else "unknown")


def amt_post(xml_body, pw, host=AMT_ADDRESS, timeout=15):
    """POST a WS-Management request with HTTP digest auth (user admin). Raises OSError when AMT does not answer."""
    url = f"http://{host}:16992/wsman"
    mgr = urllib.request.HTTPPasswordMgrWithDefaultRealm()
    mgr.add_password(None, url, "admin", pw)
    opener = urllib.request.build_opener(urllib.request.HTTPDigestAuthHandler(mgr))
    req = urllib.request.Request(url, data=xml_body.encode(), headers={"Content-Type": "application/soap+xml;charset=UTF-8"})
    try:
        return opener.open(req, timeout=timeout).read().decode(errors="replace")
    except urllib.error.HTTPError as e:
        return e.read().decode(errors="replace") or f"HTTP {e.code}"


def cmd_power(args):
    """`power state|on|off|cycle|reset`: AMT power control from titan to 10.99.0.201:16992 (WS-Management, digest auth, user admin,
    password from out/lab/amt.password). Works only once AMT is activated (`amt-activate`); until then it says so."""
    op = args[0] if args else ""
    if op not in ("state", *POWER_STATES):
        print("usage: power state|on|off|cycle|reset")
        return 2
    pw = amt_password()
    if not pw:
        print(f"AMT is not set up: {AMT_PASSWORD_FILE} does not exist. Run `amt-activate` first (it currently fails on midget with EBUSY on the LME client).")
        return 1
    if not port_open_at(AMT_ADDRESS, 16992):
        print(f"AMT does not answer on {AMT_ADDRESS}:16992: it is not activated yet (or has no address). `amt-activate` first; "
              "until then use `wake` (Wake-on-LAN), `reboot` and `poweroff`.")
        return 1
    xml_body = power_state_xml(AMT_ADDRESS) if op == "state" else power_request_xml(AMT_ADDRESS, op)
    try:
        reply = amt_post(xml_body, pw)
    except OSError as e:
        print(redact(f"AMT request failed: {e}", pw))
        return 1
    print(power_describe(parse_power_response(reply), None if op == "state" else op))
    return 0


def port_open_at(host, port, timeout=2.0):
    try:
        socket.create_connection((host, port), timeout).close()
        return True
    except OSError:
        return False


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
        "reinstall": lambda: cmd_reinstall_loop(rest),
        "qa-install": lambda: cmd_qa_install(rest),
        "unlock": lambda: cmd_unlock(),
        "login-bypass": lambda: cmd_login_bypass(rest),
        # re-attach to a reinstall whose follower died: the install runs on its own, this picks up at the current phase
        "follow": lambda: follow_install(time.time(), int(rest[rest.index("--timeout") + 1]) if "--timeout" in rest else 900, live=True, every=shot_interval(rest)),
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
        "kbench": lambda: cmd_kbench(rest),
        "kab": lambda: cmd_kab(rest),
        "amt-info": lambda: cmd_amt_info(),
        "amt-activate": lambda: cmd_amt_activate(),
        "power": lambda: cmd_power(rest),
        "ssh": lambda: (print(ssh(" ".join(rest), 120), end=""), 0)[1],
    }
    if op not in table:
        sys.exit(__doc__)
    sys.exit(table[op]())


if __name__ == "__main__":
    main()
