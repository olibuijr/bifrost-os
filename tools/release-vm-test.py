#!/usr/bin/env python3
"""Install an unchanged release ISO in a disposable UEFI VM and verify the real desktop.

Usage: tools/release-vm-test.py --iso PATH

All VM state is private to a new out/vm-runs/<stamp>-release directory. The ISO is opened read-only and the guest
has no outbound network; a loopback-only forwarded SSH connection is opened after the release hardening check.
No lab services, installer replacement, host disks, existing QEMU processes, or physical machines are involved.
Stdlib only. QEMU, OVMF, ssh and ssh-keygen are required; the ISO must include a complete offline package/app bundle.
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import secrets
import shlex
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
os.chdir(REPO)
WIDTH, HEIGHT = 1280, 720
DISK_BYTES = 24 * 1024**3
BOOT_TIMEOUT = 240
INSTALL_TIMEOUT = 1800
DESKTOP_TIMEOUT = 300

EVDEV = {
    **dict(zip("abcdefghijklmnopqrstuvwxyz", [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44])),
    **dict(zip("1234567890", [2, 3, 4, 5, 6, 7, 8, 9, 10, 11])),
}
SHIFT = 42
SHIFTED = dict(zip("!@#$%^&*()_+", "1234567890-="))
PUNCT = {" ": "spc", "-": "minus", "=": "equal", "[": "bracket_left", "]": "bracket_right", "\\": "backslash",
         ";": "semicolon", "'": "apostrophe", ",": "comma", ".": "dot", "/": "slash", "`": "grave_accent",
         ":": "semicolon", '"': "apostrophe", "<": "comma", ">": "dot", "?": "slash", "|": "backslash", "~": "grave_accent"}


def decode_guest_exec_status(status: dict) -> tuple[bool, int | None, str, str]:
    """Decode one QGA guest-exec-status response (whose output fields are base64 strings)."""
    if not isinstance(status, dict):
        raise RuntimeError(f"invalid QGA guest-exec-status response: {status!r}")
    if not status.get("exited", False):
        return False, None, "", ""
    exitcode = status.get("exitcode")
    if exitcode is not None and not isinstance(exitcode, int):
        raise RuntimeError(f"invalid QGA guest process exit code: {exitcode!r}")
    try:
        stdout = base64.b64decode(status.get("out-data", ""), validate=True).decode("utf-8", errors="replace")
        stderr = base64.b64decode(status.get("err-data", ""), validate=True).decode("utf-8", errors="replace")
    except (ValueError, TypeError) as exc:
        raise RuntimeError(f"invalid base64 output from QGA guest process: {exc}") from exc
    return True, exitcode, stdout, stderr


class Qga:
    """Small synchronous QEMU Guest Agent JSON-line client for the harness-owned guest channel."""

    def __init__(self, path: Path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(10)
        self.sock.connect(str(path))
        self.reader = self.sock.makefile("r", encoding="utf-8")
        self.writer = self.sock.makefile("w", encoding="utf-8")

    def execute(self, name: str, **arguments):
        self.writer.write(json.dumps({"execute": name, "arguments": arguments}) + "\n")
        self.writer.flush()
        while True:
            line = self.reader.readline()
            if not line:
                raise RuntimeError(f"QGA closed while waiting for {name}")
            reply = json.loads(line)
            if "return" in reply:
                return reply["return"]
            if "error" in reply:
                error = reply["error"]
                raise RuntimeError(f"QGA {name}: {error.get('desc', error)}")
            # Some agents can emit events between command replies; they are not command results.

    def run(self, script: str, *, timeout: float = 30, poll_interval: float = 0.25) -> str:
        result = self.execute("guest-exec", path="/bin/sh", arg=["-c", script], **{"capture-output": True})
        pid = result.get("pid") if isinstance(result, dict) else None
        if not isinstance(pid, int):
            raise RuntimeError(f"QGA guest-exec returned no process id: {result!r}")
        deadline = time.monotonic() + timeout
        while True:
            status = self.execute("guest-exec-status", pid=pid)
            exited, exitcode, stdout, stderr = decode_guest_exec_status(status)
            if exited:
                if exitcode != 0:
                    raise RuntimeError(f"guest command exited {exitcode}; stdout={stdout[-2000:]!r}; stderr={stderr[-2000:]!r}")
                return stdout
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"QGA guest process {pid} did not finish within {timeout} seconds")
            time.sleep(min(poll_interval, remaining))

    def close(self):
        try:
            self.reader.close()
        finally:
            try:
                self.writer.close()
            finally:
                self.sock.close()


class Qmp:
    def __init__(self, path: Path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(12)
        self.sock.connect(str(path))
        self.stream = self.sock.makefile("rw")
        self.stream.readline()  # QMP greeting
        self.command("qmp_capabilities")

    def command(self, name: str, **arguments):
        self.stream.write(json.dumps({"execute": name, "arguments": arguments}) + "\n")
        self.stream.flush()
        while True:
            line = self.stream.readline()
            if not line:
                raise RuntimeError(f"QMP closed while waiting for {name}")
            reply = json.loads(line)
            if "return" in reply:
                return reply["return"]
            if "error" in reply:
                raise RuntimeError(f"QMP {name}: {reply['error'].get('desc', reply['error'])}")

    def key(self, key: str, *modifiers: str):
        chord = [{"type": "qcode", "data": modifier} for modifier in modifiers]
        chord.append({"type": "qcode", "data": key})
        self.command("send-key", keys=chord)

    def type_text(self, value: str, delay: float = 0.025):
        for char in value:
            if char.isalpha():
                self.key(char.lower(), *(('shift',) if char.isupper() else ()))
            elif char in "0123456789":
                self.key(char)
            elif char in SHIFTED:
                base = SHIFTED[char]
                self.key(PUNCT.get(base, base), "shift")
            elif char in ":\"<>?|~":
                self.key(PUNCT[char], "shift")
            elif char in PUNCT:
                self.key(PUNCT[char])
            else:
                raise ValueError(f"QMP typing does not support {char!r}")
            time.sleep(delay)

    def tap(self, x: int, y: int):
        def axis(name, value, size):
            value = max(0, min(size - 1, value))
            return {"type": "abs", "data": {"axis": name, "value": int(value * 32767 / max(size - 1, 1))}}
        events = [axis("x", x, WIDTH), axis("y", y, HEIGHT), {"type": "btn", "data": {"button": "left", "down": True}}]
        self.command("input-send-event", events=events)
        time.sleep(0.08)
        self.command("input-send-event", events=[{"type": "btn", "data": {"button": "left", "down": False}}])

    def screenshot(self, path: Path):
        self.command("screendump", filename=str(path), format="png")

    def close(self):
        self.stream.close()
        self.sock.close()


def command(args, timeout=30, *, check=True, input_text=None):
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, input=input_text)
    if check and result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {shlex.join(args)}\n{result.stderr[-2000:]}")
    return result


def choose_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def ssh_args(port: int, key: Path, user: str = "root"):
    return ["ssh", "-i", str(key), "-p", str(port), "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no",
            "-o", "BatchMode=yes", "-o", "ConnectTimeout=4", "-o", "LogLevel=ERROR", f"{user}@127.0.0.1"]


def remote(port: int, key: Path, command_text: str, timeout=30, *, user="root", check=True, input_text=None):
    return command(ssh_args(port, key, user) + [command_text], timeout, check=check, input_text=input_text)


def wait_until(label: str, timeout: float, predicate, interval=2.0):
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except Exception as exc:
            last_error = str(exc)
        time.sleep(interval)
    raise TimeoutError(f"timed out waiting for {label}" + (f"; last error: {last_error}" if last_error else ""))


def parse_state(text: str):
    parsed = {"button": [], "field": []}
    for line in text.splitlines():
        key, sep, value = line.partition("=")
        if not sep:
            continue
        if key in ("button", "field"):
            parsed[key].append(value.split())
        else:
            parsed[key] = value
    return parsed


class InstallerFailure(RuntimeError):
    """The live installer reported a terminal failure state."""


class ReleaseVmTest:
    def __init__(self, iso: Path, out: Path):
        self.iso = iso
        self.out = out
        self.disk = out / "target.raw"
        self.vars = out / "ovmf-vars.fd"
        self.qmp_path = out / "qmp.sock"
        self.qga_path = out / "qga.sock"
        if len(str(self.qmp_path)) >= 104:
            self.qmp_path = Path("/tmp") / f"bifrost-release-{out.name[-16:]}.sock"
        if len(str(self.qga_path)) >= 104:
            self.qga_path = Path("/tmp") / f"bifrost-qga-{out.name[-16:]}.sock"
        self.port = choose_port()
        self.key = out / "diagnostic_ed25519"
        self.pub = self.key.with_suffix(".pub")
        self.process: subprocess.Popen | None = None
        self.qmp: Qmp | None = None
        self.qga: Qga | None = None
        self.started = time.monotonic()
        self.phases: dict[str, float] = {}
        self.checks: dict[str, object] = {}
        self.notes: list[str] = []
        self.iso_sha256: str | None = None
        self.user = "qa" + secrets.token_hex(4)
        self.password = "".join(secrets.choice("abcdefghijklmnopqrstuvwxyz0123456789") for _ in range(32))
        self.ok = False

    def phase(self, name: str, fn):
        start = time.monotonic()
        result = fn()
        self.phases[name] = round(time.monotonic() - start, 1)
        self.write_report()
        print(f"[{time.monotonic() - self.started:7.1f}s] phase {name} completed ({self.phases[name]:.1f}s)", flush=True)
        return result

    def save_shot(self, name: str):
        path = self.out / f"{name}.png"
        self.qmp.screenshot(path)
        return path

    def qmp_until(self):
        return wait_until("QMP socket", 30, lambda: self._connect_qmp(), 0.25)

    def _connect_qmp(self):
        try:
            self.qmp = Qmp(self.qmp_path)
            return self.qmp
        except OSError:
            return None

    def host_run(self):
        if not self.iso.is_file() or self.iso.is_symlink():
            raise ValueError(f"--iso must be an existing regular, non-symlink file: {self.iso}")
        if self.iso.stat().st_size < 64 * 1024 * 1024:
            raise ValueError("ISO is unexpectedly small")
        self.iso_sha256 = self._iso_hash()
        if self.disk.exists():
            raise FileExistsError(f"refusing to overwrite VM disk: {self.disk}")
        code = Path("/usr/share/edk2/x64/OVMF_CODE.4m.fd")
        template = Path("/usr/share/edk2/x64/OVMF_VARS.4m.fd")
        qemu = shutil.which("qemu-system-x86_64")
        if not qemu or not code.is_file() or not template.is_file():
            raise FileNotFoundError("qemu-system-x86_64 or OVMF 4m firmware is unavailable")
        fd = os.open(self.disk, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        try:
            os.ftruncate(fd, DISK_BYTES)
        finally:
            os.close(fd)
        fd = os.open(self.vars, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        try:
            with template.open("rb") as source, os.fdopen(fd, "wb") as target:
                shutil.copyfileobj(source, target)
        except Exception:
            os.close(fd)
            raise
        command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "release-vm-test", "-f", str(self.key)], 15)
        argv = [qemu, "-name", f"bifrost-release-test-{self.out.name}", "-machine", "q35,accel=kvm", "-cpu", "host", "-smp", "4", "-m", "4096",
                "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
                "-drive", f"if=pflash,format=raw,unit=1,file={self.vars}",
                "-drive", f"file={self.iso},media=cdrom,if=none,id=cd,readonly=on", "-device", "ide-cd,drive=cd,bootindex=2",
                "-device", "virtio-scsi-pci,id=scsi0", "-drive", f"file={self.disk},if=none,id=target,format=raw", "-device", "scsi-hd,drive=target,bus=scsi0.0,bootindex=1",
                "-device", f"virtio-vga,xres={WIDTH},yres={HEIGHT}", "-device", "virtio-keyboard-pci", "-device", "virtio-tablet-pci",
                # The existing Arch ISO includes qemu-guest-agent; Arch's udev rule starts it when this named port appears.
                "-chardev", f"socket,id=qga,path={self.qga_path},server=on,wait=off",
                "-device", "virtio-serial-pci,id=qga-serial",
                "-device", "virtserialport,chardev=qga,name=org.qemu.guest_agent.0",
                "-netdev", f"user,id=net0,restrict=on,hostfwd=tcp:127.0.0.1:{self.port}-:22", "-device", "virtio-net-pci,netdev=net0",
                "-display", "none", "-qmp", f"unix:{self.qmp_path},server=on,wait=off", "-serial", f"file:{self.out / 'serial.log'}"]
        qemu_log = (self.out / "qemu.log").open("wb")
        self.process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=qemu_log, stderr=subprocess.STDOUT, start_new_session=True)
        self.out.mkdir(exist_ok=True)
        self.write_report()
        self.qmp_until()
        return self.run_guest()

    def run_guest(self):
        self.phase("release_iso_boot", self.boot_and_instrument)
        self.phase("installer_ui", self.install_via_ui)
        self.phase("installed_boot", self.reboot_to_disk)
        self.phase("desktop", self.verify_desktop)
        self.ok = True

    def boot_and_instrument(self):
        wait_until("QEMU Guest Agent", BOOT_TIMEOUT, self._guest_agent_ready, 2)
        hardening = self.qga.run(
            "if [ -e /etc/bifrost/lab.token ] || [ -e /root/.ssh/authorized_keys ] || "
            "[ -e /usr/lib/systemd/system/bifrost-lab.service ]; then "
            "echo RELEASE_HARDENING_FAIL; exit 31; fi; echo RELEASE_HARDENING_OK",
            timeout=20,
        )
        if "RELEASE_HARDENING_OK" not in hardening:
            raise RuntimeError("could not prove the unchanged live ISO is release-hardened before instrumentation")
        self.checks["live_release_hardening"] = "passed before diagnostic key injection"
        egress = self.qga.run(
            "if curl -fsS --connect-timeout 5 --max-time 8 -o /dev/null "
            "https://bifrost.olibuijr.com/arch/x86_64/bifrost.db; then "
            "echo EGRESS_AVAILABLE; exit 32; else echo EGRESS_BLOCKED; fi",
            timeout=20,
        )
        if "EGRESS_BLOCKED" not in egress:
            raise RuntimeError("release VM had outbound network access; offline install isolation was not proven")
        self.checks["guest_outbound_network"] = "blocked"
        public_key = self.pub.read_text().strip()
        inject = ("install -d -m 700 /root/.ssh && printf '%s\\n' " + shlex.quote(public_key) +
                  " > /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys && systemctl start sshd")
        self.qga.run(inject, timeout=20)
        wait_until("diagnostic SSH", 90, self._ssh_ready, 2)
        result = remote(self.port, self.key, "test ! -e /etc/bifrost/lab.token && "
                        "test ! -e /usr/lib/systemd/system/bifrost-lab.service", timeout=20)
        if result.returncode:
            raise RuntimeError("release live system changed its lab-token/service hardening state after instrumentation")
        self.save_shot("01-live-release")
        return True

    def _guest_agent_ready(self):
        try:
            if self.qga is None:
                self.qga = Qga(self.qga_path)
            self.qga.execute("guest-ping")
            return True
        except Exception:
            if self.qga is not None:
                try:
                    self.qga.close()
                except Exception:
                    pass
                self.qga = None
            return False

    def _ssh_ready(self):
        result = remote(self.port, self.key, "echo ready", timeout=8, check=False)
        return result.returncode == 0 and result.stdout.strip() == "ready"

    def guest(self, text: str, timeout=30, check=True):
        return remote(self.port, self.key, text, timeout, check=check)

    def state(self):
        result = self.guest("cat /run/bifrost-installer/state", timeout=8)
        return parse_state(result.stdout)

    def wait_page(self, page: str, timeout=90):
        return wait_until(f"installer page {page}", timeout, lambda: (s := self.state()).get("screen") == page and s, 1)

    def tap_named(self, kind: str, name: str):
        state = self.state()
        size = state.get("size", "1280 720").split()
        sw, sh = float(size[0]), float(size[1])
        rows = state.get(kind, [])
        for row in rows:
            if row and row[0] == name and len(row) >= 5:
                x, y, w, h = map(float, row[1:5])
                self.qmp.tap(int((x + w / 2) * WIDTH / sw), int((y + h / 2) * HEIGHT / sh))
                time.sleep(0.5)
                return
        raise RuntimeError(f"installer state has no {kind}={name}: {state}")

    def key_fifo(self, *codes: int):
        lines = "".join(f"key {code} 1\\nkey {code} 0\\n" for code in codes)
        self.guest("printf '%b' " + shlex.quote(lines) + " > /run/bifrost-input", timeout=10)
        time.sleep(0.12)

    def fifo_text(self, text: str):
        codes = []
        for char in text:
            if char not in EVDEV:
                raise ValueError(f"installer FIFO typing supports only lowercase ASCII and digits: {char!r}")
            codes.append(EVDEV[char])
        self.key_fifo(*codes)

    def install_via_ui(self):
        wait_until("installer UI ready", 180, lambda: self._installer_ready(), 3)
        self.qmp.key("f1", "ctrl", "alt")
        time.sleep(0.5)
        self.save_shot("02-welcome")
        self.tap_named("button", "next")
        self.wait_page("language")
        self.save_shot("03-language")
        self.tap_named("button", "next")
        self.wait_page("account")
        # Full name autofills the unique lowercase username. Drive password fields through the real input FIFO.
        self.tap_named("field", "name")
        self.fifo_text(self.user)
        self.tap_named("field", "password")
        self.fifo_text(self.password)
        self.tap_named("field", "verify")
        self.fifo_text(self.password)
        self.save_shot("04-account")
        self.tap_named("button", "next")
        self.wait_page("look")
        self.save_shot("05-look")
        self.tap_named("button", "next")
        disk_page = self.wait_page("disk")
        self.save_shot("06-disk")
        devices = self.guest("lsblk -dnpo NAME,TYPE | awk '$2==\"disk\" {print $1}'").stdout.split()
        if len(devices) != 1:
            raise RuntimeError(f"expected exactly the one disposable VM disk, found {devices}")
        # The QEMU harness presents only the newly created SCSI target as a disk; assert installer selected it.
        disks = [row for row in disk_page.get("button", []) if row[0].startswith("disk")]
        if not disks:
            raise RuntimeError("installer did not expose the VM target disk in its state")
        self.tap_named("button", "install")
        self.wait_page("confirm")
        self.save_shot("07-confirm")
        self.tap_named("button", "erase")
        self.wait_install_finished(INSTALL_TIMEOUT)
        self.save_shot("08-done")
        self.verify_target_clean_and_seed_key()
        # The disk is boot index 1 and the ISO is index 2, so the UI's reboot enters the installed system.
        self.tap_named("button", "restart")
        return True

    def _installer_ready(self):
        result = remote(self.port, self.key, "test -p /run/bifrost-input && test -s /run/bifrost-installer/state && echo ready", timeout=8, check=False)
        return result.returncode == 0 and "ready" in result.stdout

    def wait_install_finished(self, timeout: float):
        deadline = time.monotonic() + timeout
        last_state: dict[str, object] = {}
        last_error = ""
        while time.monotonic() < deadline:
            try:
                state = self.state()
                last_state = state
                self.checks["installer_state"] = state
                if state.get("screen") == "failed":
                    self.capture_installer_failure(state, "installer entered its failed page")
                    raise InstallerFailure(f"installer failed: {state}")
                if state.get("screen") == "done":
                    return state
            except InstallerFailure:
                raise
            except Exception as exc:
                last_error = str(exc)
            time.sleep(min(5, max(0.0, deadline - time.monotonic())))
        self.capture_installer_failure(last_state, "installer did not reach Done before timeout")
        raise TimeoutError("installer did not reach Done within " + str(timeout) + " seconds"
                           + (f"; last state error: {last_error}" if last_error else f"; last state: {last_state}"))

    def capture_installer_failure(self, state: dict[str, object], reason: str):
        self.checks["installer_failure_reason"] = reason
        self.checks["installer_state_at_failure"] = state
        try:
            journal = self.guest("journalctl -b -u bifrost-installer --no-pager -n 100", timeout=20, check=False)
            body = journal.stdout + ("\n[journal stderr]\n" + journal.stderr if journal.stderr else "")
            path = self.out / "installer-journal.log"
            path.write_text(body)
            self.checks["installer_journal_file"] = str(path)
            if journal.returncode:
                self.checks["installer_journal_exit"] = journal.returncode
        except Exception as exc:
            self.checks["installer_journal_error"] = str(exc)
        self.write_report()

    def verify_target_clean_and_seed_key(self):
        # Keep the live system and its SSH access until all release-hardening checks pass.
        user = shlex.quote(self.user)
        script = ("set -eu; disk=$(lsblk -dnpo NAME,TYPE | awk '$2==\"disk\" {print $1}'); "
                  "set -- $disk; test \"$#\" -eq 1; disk=$1; "
                  "roots=$(lsblk -nrpo NAME,FSTYPE \"$disk\" | awk '$2==\"btrfs\" {print $1}'); "
                  "set -- $roots; test \"$#\" -eq 1; root=$1; mkdir -p /mnt; mount -t btrfs -o subvol=@system \"$root\" /mnt; "
                  "trap 'umount -R /mnt || true' EXIT; test -f /mnt/etc/os-release; "
                  "test ! -e /mnt/etc/bifrost/lab.token; test ! -e /mnt/root/.ssh/authorized_keys; "
                  "test ! -e /mnt/usr/lib/systemd/system/bifrost-lab.service; "
                  "mount -t btrfs -o subvol=@home \"$root\" /mnt/home; test -d /mnt/home/" + user + "; "
                  "install -d -m 700 /mnt/home/" + user + "/.ssh; "
                  "install -m 600 /root/.ssh/authorized_keys /mnt/home/" + user + "/.ssh/authorized_keys; "
                  "ids=$(awk -F: -v u=" + user + " '$1==u {print $3 \":\" $4}' /mnt/etc/passwd); test -n \"$ids\"; "
                  "chown -R \"$ids\" /mnt/home/" + user + "/.ssh; "
                  "echo installed-release-clean")
        checks = self.guest(script, timeout=30)
        if "installed-release-clean" not in checks.stdout:
            raise RuntimeError("could not prove installed release is clean before adding VM-only diagnostic key")

    def reboot_to_disk(self):
        # Tapping the live UI Restart button requests its normal reboot path; the QEMU child is still ours exclusively.
        wait_until("VM reboot after install", BOOT_TIMEOUT, lambda: self._ssh_is_installed(), 3)
        return True

    def _ssh_is_installed(self):
        result = remote(self.port, self.key, "test ! -d /run/archiso && echo installed", timeout=8, check=False,
                        user=self.user)
        return result.returncode == 0 and "installed" in result.stdout

    def installed(self, text: str, timeout=30, *, check=True, sudo=False):
        if sudo:
            command_text = "sudo -S -p '' sh -c " + shlex.quote(text)
            return remote(self.port, self.key, command_text, timeout, user=self.user, check=check, input_text=self.password + "\n")
        return remote(self.port, self.key, text, timeout, user=self.user, check=check)

    def control(self, line: str):
        # the shell's control socket is a Unix socket served to root and the installed account (the user this runs as)
        script = ("import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(8); s.connect('/run/bifrost/shell-control.sock'); s.sendall(" +
                  repr((line + "\n").encode()) + "); d=b''\nwhile True:\n c=s.recv(65536)\n if not c: break\n d+=c\nprint(d.decode(),end='')")
        result = self.installed("python3 -c " + shlex.quote(script), timeout=15)
        return result.stdout

    def verify_desktop(self):
        package = wait_until("installed SSH and shell startup", DESKTOP_TIMEOUT, lambda: self._desktop_ready(), 3)
        self.checks["installed_package_version"] = package
        apps = self.installed("cat /var/lib/springboard/apps/installed.json 2>/dev/null || true", timeout=20, check=False).stdout
        self.checks["installed_store_apps"] = [app for app in ("is.olibuijr.browser.desktop", "is.olibuijr.files.desktop") if app in apps]
        if "is.olibuijr.browser.desktop" not in apps or "is.olibuijr.files.desktop" not in apps:
            raise RuntimeError("the default Browser and Files App Store installs are missing")
        state = wait_until("shell lock screen", 60, lambda: self.control("state"), 2)
        self.save_shot("09-lock-screen")
        if "screen=lock" not in state:
            raise RuntimeError(f"installed shell did not reach its lock screen: {state[:500]}")
        self.qmp.key("ret")
        time.sleep(1)
        self.qmp.type_text(self.password)
        self.qmp.key("ret")
        state = wait_until("unlocked home screen", 90, lambda: self.control("state") if "screen=home" in self.control("state") else "", 2)
        self.save_shot("10-desktop-home")
        self.checks["desktop_state"] = next((line for line in state.splitlines() if line.startswith("screen=")), "")
        self.write_report()
        return True

    def _desktop_ready(self):
        result = self.installed("set -eu; systemctl is-active --quiet springboard; pgrep -x bifrost-wm >/dev/null; "
                                "pacman -Q springboard-shell | awk '{print $2}'", timeout=20, check=False, sudo=True)
        if result.returncode:
            return False
        lines = result.stdout.splitlines()
        version = next((line.strip() for line in reversed(result.stdout.splitlines()) if line.strip()), "")
        match = re.match(r"^(\d+)\.(\d+)\.(\d+)", version)
        if not match:
            return False
        if tuple(int(part) for part in match.groups()) < (0, 8, 0):
            raise RuntimeError(f"installed springboard-shell must be at least 0.8.0, got {version}")
        return version

    def write_report(self):
        report = {"ok": self.ok, "iso": str(self.iso), "iso_sha256": self.iso_sha256 or "not yet computed", "run_dir": str(self.out),
                  "disk": str(self.disk), "disk_bytes": DISK_BYTES, "username": self.user,
                  "phase_seconds": self.phases, "checks": self.checks,
                  "elapsed_seconds": round(time.monotonic() - self.started, 1), "notes": self.notes}
        (self.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        status = "PASS" if self.ok else "INCOMPLETE/FAILED"
        rows = "\n".join(f"- `{name}`: {value}s" for name, value in self.phases.items()) or "- No phases completed."
        checks = "\n".join(f"- `{name}`: `{value}`" for name, value in self.checks.items()) or "- No post-install checks completed."
        screenshots = "\n".join(f"- `{p.name}`" for p in sorted(self.out.glob("*.png"))) or "- None captured."
        notes = "\n".join(f"- {note}" for note in self.notes) or "- None."
        text = (f"# Release ISO VM test: {status}\n\n- ISO: `{self.iso}`\n- SHA-256: `{report['iso_sha256']}`\n"
                f"- Run directory: `{self.out}`\n- Disposable VM disk: `{self.disk}` ({DISK_BYTES // 1024**3} GiB sparse raw)\n"
                f"- Test account: `{self.user}` (generated password retained only in this process)\n"
                f"- Final desktop reached: `{self.ok}`\n\n## Phases\n\n{rows}\n\n## Checks\n\n{checks}\n\n## Screenshots\n\n{screenshots}\n\n## Notes\n\n{notes}\n")
        (self.out / "report.md").write_text(text)

    def _iso_hash(self):
        if self.iso_sha256 is not None:
            return self.iso_sha256
        import hashlib
        digest = hashlib.sha256()
        try:
            with self.iso.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
            return digest.hexdigest()
        except OSError:
            return "unavailable"

    def close(self):
        # Quit only the child started by this instance; never match or signal unrelated QEMU processes.
        if self.qmp:
            try:
                self.qmp.command("quit")
            except Exception:
                pass
        if self.qga:
            try:
                self.qga.close()
            except Exception:
                pass
        if self.qmp:
            try:
                self.qmp.close()
            except Exception:
                pass
        if self.process:
            try:
                self.process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
        if self.qmp_path.parent == Path("/tmp"):
            self.qmp_path.unlink(missing_ok=True)
        if self.qga_path.parent == Path("/tmp"):
            self.qga_path.unlink(missing_ok=True)
        else:
            self.qga_path.unlink(missing_ok=True)
        self.key.unlink(missing_ok=True)
        self.pub.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iso", required=True, type=Path, help="unchanged release ISO to install")
    args = parser.parse_args()
    iso = args.iso.expanduser().resolve(strict=True)
    stamp = time.strftime("%Y%m%d-%H%M%S") + "-release-" + secrets.token_hex(3)
    out = REPO / "out" / "vm-runs" / stamp
    out.mkdir(parents=True, exist_ok=False)
    vm = ReleaseVmTest(iso, out)
    try:
        vm.host_run()
    except Exception as exc:
        vm.notes.append(f"{type(exc).__name__}: {exc}")
        vm.ok = False
        print(f"release VM test failed: {exc}", file=sys.stderr)
    finally:
        vm.close()
        vm.write_report()
    print(f"report: {out / 'report.md'}")
    return 0 if vm.ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
