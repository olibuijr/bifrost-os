#!/usr/bin/env python3
"""Boot benchmark: time every step of the installer (live) boot and the installed-system boot in the OVMF VM.

Steps come from one clock where possible:
  firmware, loader      the BifrOSt EFI loader's LoaderTimeInitUSec/LoaderTimeExecUSec (systemd-analyze)
  kernel, initrd, userspace   systemd-analyze
  milestones            /run/bifrost/boot-marks, ms since kernel start (CLOCK_BOOTTIME): installer-display,
                        installer-first-ui, installer-faded; session-start, compositor-start, compositor-ready,
                        shell-launch, desktop-first-ui, desktop-faded
  modesets              display programming after the loader: lines where our KMS code selected (not inherited)
                        a mode, and wlroots "Modesetting" lines
No ISO is rebuilt: --inject copies the working-tree build into the VM (live: tools/lab-push.sh feeds lab-sync;
installed: scp into the installed system, then reboot it) before measuring.

usage: boot-bench.py live|installed [--runs N] [--inject] [--esp IMG] [--label TEXT] [--record]
"""
import argparse, importlib.util, json, os, statistics, subprocess, sys, time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(REPO)
spec = importlib.util.spec_from_file_location("vmc", "tools/vm-cycle.py")
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)

MILESTONES = {
    "live": ["installer-display", "installer-first-ui", "installer-faded"],
    "installed": ["session-start", "compositor-start", "compositor-ready", "shell-launch", "desktop-first-ui", "desktop-faded"],
}


def stop_vm():
    """Power off cleanly: a killed VM counts as a shell start that never stayed up, and three of those trip the
    shell's crash-loop latch (exit 70) on the next boot."""
    vm.ssh("rm -f /var/lib/springboard/boot_attempts; sync; systemctl poweroff", 10)
    gone = lambda: subprocess.run(["pgrep", "-f", "^qemu-system-x86_64 -name BifrOSt"], capture_output=True).returncode != 0
    if not vm.wait_for(gone, 30, 0.5):
        subprocess.run(["pkill", "-f", "^qemu-system-x86_64 -name BifrOSt"], capture_output=True)
        time.sleep(1.5)


def boot(mode, esp):
    env = {**os.environ, "HEADLESS": "1"}
    if mode == "installed":
        env.update(ISO_NONE="1", KEEP_VARS="1")
    else:
        env["ISO"] = latest_iso()
        if esp:
            env["ISO_BOOT_IMG"] = esp
    subprocess.run(["tools/arch-vm-iso.sh"], env=env, capture_output=True, timeout=60)


def latest_iso():
    import glob
    return sorted(glob.glob("out/iso-lab-*/*.iso"), key=os.path.getmtime)[-1]


def marks():
    out = {}
    for line in vm.ssh("cat /run/bifrost/boot-marks 2>/dev/null").splitlines():
        p = line.split()
        if len(p) == 2:
            out.setdefault(p[1], float(p[0]))
    return out


def analyze():
    """systemd-analyze's per-stage seconds, parsed from its one-line summary."""
    line = vm.ssh("systemd-analyze 2>/dev/null | head -1")
    out = {}
    for part in line.replace("Startup finished in", "").split("+"):
        part = part.strip().split("=")[0].strip()
        if "(" not in part:
            continue
        val, name = part.split("(", 1)
        out[name.rstrip(")").strip()] = seconds(val.strip())
    return out


def seconds(text):
    total = 0.0
    for tok in text.split():
        for unit, k in (("ms", 0.001), ("min", 60.0), ("s", 1.0), ("us", 1e-6)):
            if tok.endswith(unit) and tok[: -len(unit)].replace(".", "").isdigit():
                total += float(tok[: -len(unit)]) * k
                break
    return round(total, 3)


def modesets():
    n = vm.ssh("journalctl -b --no-pager -o cat | grep -acE 'Modesetting with|display mode .*\\(selected\\)'").strip()
    return int(n or 0)


def inject(mode):
    if mode == "live":
        subprocess.run(["tools/lab-push.sh", "--skip-cargo"], check=True, capture_output=True)
        return
    files = [
        ("springboard/target/fast/shell", "/usr/bin/springboard", "755"),
        ("springboard/target/fast/bootweave", "/usr/bin/bifrost-bootweave", "755"),
        ("packaging/arch/wayland/session", "/usr/lib/bifrost/session", "755"),
        ("packaging/archiso/bootweave.efi", "/boot/EFI/BIFROST/BOOTx64.efi", "644"),
        ("packaging/archiso/bootweave.efi", "/boot/EFI/BOOT/BOOTx64.efi", "644"),
        ("packaging/arch/springboard.service", "/usr/lib/systemd/system/springboard.service", "644"),
        ("packaging/arch/seatd-early.conf", "/usr/lib/systemd/system/seatd.service.d/bifrost-early.conf", "644"),
        ("packaging/arch/initcpio/install/bifrost-mounts", "/usr/lib/initcpio/install/bifrost-mounts", "644"),
        ("packaging/arch/initcpio/hooks/bifrost-mounts", "/usr/lib/initcpio/hooks/bifrost-mounts", "644"),
    ]
    for src, dst, perm in files:
        subprocess.run(vm.SCP + [src, f"root@127.0.0.1:/tmp/inject"], check=True, capture_output=True, timeout=60)
        vm.ssh(f"install -D -m{perm} /tmp/inject {dst}")
    vm.ssh("mkdir -p /usr/lib/systemd/system/seatd.service.d; systemctl daemon-reload; mkinitcpio -P >/dev/null 2>&1; sync", 180)


def one_run(mode, esp, done):
    stop_vm()
    t0 = time.time()
    boot(mode, esp)
    if not vm.wait_for(lambda: vm.ssh("echo up").strip() == "up", 150, 1):
        return {"error": "no ssh"}
    vm.wait_for(lambda: done in marks(), 60, 0.5)
    time.sleep(1)
    r = {"wall_s": round(time.time() - t0, 2), "stages_s": analyze(), "marks_ms": marks(), "modesets_after_loader": modesets()}
    vm.sh(vm.QMP + ["shot", os.path.join(REPO, f"out/boot-bench-{mode}.png")], timeout=20)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["live", "installed"])
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--inject", action="store_true")
    ap.add_argument("--esp", help="live: boot this FAT ESP image (the new ISO layout) before the ISO")
    ap.add_argument("--label", default="")
    ap.add_argument("--record", action="store_true", help="append the medians to docs/perf-history.jsonl")
    a = ap.parse_args()
    done = MILESTONES[a.mode][-1]
    if a.inject:
        if a.mode == "installed":
            stop_vm()
            boot("installed", None)
            vm.wait_for(lambda: vm.ssh("echo up").strip() == "up", 150, 1)
        inject(a.mode)
    runs = [one_run(a.mode, a.esp, done) for _ in range(a.runs)]
    good = [r for r in runs if "error" not in r]
    keys = sorted({k for r in good for k in r["stages_s"]})
    med = {f"boot_{a.mode}_{k}_s": statistics.median(r["stages_s"].get(k, 0) for r in good) for k in keys}
    for m in MILESTONES[a.mode]:
        vals = [r["marks_ms"][m] for r in good if m in r["marks_ms"]]
        if vals:
            med[f"boot_{a.mode}_{m}_ms"] = statistics.median(vals)
    med[f"boot_{a.mode}_modesets"] = max((r["modesets_after_loader"] for r in good), default=-1)
    print(json.dumps({"runs": runs}, indent=1) if os.environ.get("VERBOSE") else "")
    print(f"{a.mode} boot, {len(good)}/{len(runs)} runs, medians:")
    for k, v in med.items():
        print(f"  {k:42} {v}")
    if a.record and good:
        sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
        ver = next((l.split('"')[1] for l in open("springboard/Cargo.toml") if l.startswith("version")), "")
        with open("docs/perf-history.jsonl", "a") as f:
            f.write(json.dumps({"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "version": ver, "host": "titan-vm",
                                "label": a.label, "results": med}) + "\n")
    return 0 if good else 1


if __name__ == "__main__":
    sys.exit(main())
