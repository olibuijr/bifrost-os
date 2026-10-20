#!/usr/bin/env python3
"""Health evaluation and repeatable frame-time benchmark for a running x86 Arch VM.

Example:
  ARCH_VM_DIR=$PWD/out/arch-vm ARCH_VM_SSH_PORT=2250 \
    python3 tools/arch-vm-bench.py --out out/benchmarks/desktop-baseline.json
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import fps  # noqa: E402


def run(args, *, timeout=30):
    p = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if p.returncode:
        raise RuntimeError(f"command failed ({p.returncode}): {' '.join(args)}\n{p.stderr.strip()}")
    return p.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--vm-dir", default=os.environ.get("ARCH_VM_DIR", str(TOOLS.parent / "out/arch-vm")))
    ap.add_argument("--ssh-port", type=int, default=int(os.environ.get("ARCH_VM_SSH_PORT", "2250")))
    ap.add_argument("--width", type=int, default=int(os.environ.get("ARCH_VM_W", "1280")))
    ap.add_argument("--height", type=int, default=int(os.environ.get("ARCH_VM_H", "720")))
    ap.add_argument("--qmp", help="QMP socket (defaults to VM_DIR/qmp.sock)")
    ap.add_argument("--pid-file", help="QEMU pid file (defaults to VM_DIR/qemu.pid)")
    ap.add_argument("--out", help="write JSON result (otherwise print only)")
    a = ap.parse_args()

    vm_dir = Path(a.vm_dir).resolve()
    key = vm_dir / "id_ed25519"
    qmp_socket = Path(a.qmp).resolve() if a.qmp else vm_dir / "qmp.sock"
    pid_file = Path(a.pid_file).resolve() if a.pid_file else vm_dir / "qemu.pid"
    if not key.is_file() or not qmp_socket.exists():
        raise SystemExit(f"missing VM SSH key or QMP socket under {vm_dir}")

    ssh_prefix = ["ssh", "-i", str(key), "-p", str(a.ssh_port), "-o", "StrictHostKeyChecking=no",
                  "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5",
                  "-o", "BatchMode=yes", "root@127.0.0.1"]

    def guest(command):
        return run(ssh_prefix + [command], timeout=45).strip()

    def control(line):
        # the shell's control socket is a Unix socket for root; the guest python reads the line from argv
        body = ("import socket,sys; s=socket.socket(socket.AF_UNIX); s.settimeout(3); s.connect('/run/bifrost/shell-control.sock'); "
                "s.sendall((sys.argv[1] + chr(10)).encode()); d=b''\n"
                "while True:\n c=s.recv(65536)\n if not c: break\n d+=c\nprint(d.decode(errors='replace'),end='')")
        return run(ssh_prefix + [f"python3 -c {shlex.quote(body)} {shlex.quote(line)}"], timeout=10).strip()

    def qmp(*args):
        return run([sys.executable, str(TOOLS / "arch-vm-qmp.py"), str(qmp_socket), str(a.width), str(a.height), *map(str, args)], timeout=40)

    def state():
        return dict(line.split("=", 1) for line in control("state").splitlines() if "=" in line)

    service_text = guest("systemctl show springboard.service -p ActiveState -p NRestarts")
    service = dict(line.split("=", 1) for line in service_text.splitlines() if "=" in line)
    initial_state = state()
    if initial_state.get("screen") == "lock":
        qmp("swipe", a.width // 2, int(a.height * 0.88), a.width // 2, int(a.height * 0.15), 350)
        time.sleep(1)
    home_state = state()
    if home_state.get("screen") != "home":
        qmp("key", "meta_l")
        time.sleep(1)
        home_state = state()

    control("trace off")
    started = time.time()
    control("trace on")
    swipe_results = []
    y = a.height // 2
    workload = [(a.width - 120, y, 120, y, 300), (120, y, a.width - 120, y, 300),
                (a.width - 120, y, 120, y, 500), (120, y, a.width - 120, y, 500)] * 3
    for x0, y0, x1, y1, ms in workload:
        qmp("swipe", x0, y0, x1, y1, ms)
        swipe_results.append(ms)
        time.sleep(ms / 1000 + 0.35)
    control("trace off")
    log = guest(f"journalctl -u springboard.service --since @{int(started)} --no-pager -o cat")
    frames = fps.summarize(fps.parse_frames(log))

    memory = guest("free -b; printf '\\nSHELL_RSS_KB='; ps -o rss= -C springboard | awk '{s+=$1} END {print s+0}'; df -B1 --output=size,used,avail / | tail -1")
    memory_lines = memory.splitlines()
    qemu_rss_kib = None
    try:
        pid = int(pid_file.read_text().strip())
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                qemu_rss_kib = int(line.split()[1])
    except (OSError, ValueError):
        pass

    evaluation = {
        "service_active": service.get("ActiveState") == "active",
        "service_restarts_zero": service.get("NRestarts") == "0",
        "home_reached": home_state.get("screen") == "home",
        "benchmark_frames_present": bool(frames and frames["frames"] > 0),
    }
    result = {
        "vm_dir": str(vm_dir), "ssh_port": a.ssh_port, "display": {"width": a.width, "height": a.height},
        "service": service, "initial_state": initial_state, "post_unlock_state": home_state,
        "evaluation": evaluation, "passed": all(evaluation.values()),
        "frame_benchmark": frames, "swipes_ms": swipe_results,
        "guest_memory_disk_raw": memory_lines, "qemu_rss_kib": qemu_rss_kib,
        "renderer_log_lines": [line for line in log.splitlines() if "renderer" in line.lower() or "display" in line.lower()],
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    encoded = json.dumps(result, indent=2)
    if a.out:
        out = Path(a.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(encoded + "\n")
    print(encoded)
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
