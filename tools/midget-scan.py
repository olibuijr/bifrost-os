#!/usr/bin/env python3
"""Scripted bug scan of Midget for the wave loop: no model in the loop, one short digest out.

usage: tools/midget-scan.py OUT_DIR [--apps id,id,...] [--settle S]

For each installed system app (Craft apps, FilmCraft, Deildu and camera apps skipped): open it, wait, take a
screenshot (OUT_DIR/shots/<app>.png, 1280 px), quit it. Then collect, for the scan's own time window only: journal
lines at warning or worse and app/shell lines with error words (known lab-tool noise dropped), core dumps, slow
frames per screen, and the shell's state. Writes OUT_DIR/digest.md (counts, top distinct messages, per-app notes)
and OUT_DIR/journal.txt (the filtered lines). Triage reads the digest and the screenshots, not raw logs.
Every call goes through tools/midget.py (bounded); nothing is typed into apps and no file is changed on Midget.
"""
import collections, os, re, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MIDGET = [sys.executable, os.path.join(ROOT, "tools/midget.py")]
SKIP = re.compile(r"^ai\.storyteller\.|filmcraft|deildu|photobooth|imagecapture|installed\.json")
# lab-tool noise: each midget.py input action makes and removes a temporary uinput device; ssh sessions
NOISE = re.compile(r"Could not revoke evdev|sshd-session|pam_unix|systemd-logind|session-\d+\.scope|BifrOSt remote pointer|"
                   r"runuser|Started Session|frame draw|input \[|gpu present:")
ERRWORD = re.compile(r"\b(error|panic|panicked|failed|fatal|crash|abort|segfault|unwrap|overflow|timed out|refused)\b", re.I)


def midget(*args, timeout=120):
    try:
        r = subprocess.run(MIDGET + list(args), capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return 124, "timeout"


def ssh(cmd, timeout=120):
    return midget("ssh", cmd, timeout=timeout)[1]


def norm(line):
    """A journal line without its timestamp, pid and numbers, so repeats group together."""
    line = re.sub(r"^\w{3} \d+ [\d:]+ \S+ ", "", line)
    line = re.sub(r"\[\d+\]", "", line)
    return re.sub(r"\d+(\.\d+)?", "N", line).strip()


def main(argv):
    if not argv or argv[0].startswith("-"):
        print(__doc__)
        return 2
    out = argv[0]
    settle = float(argv[argv.index("--settle") + 1]) if "--settle" in argv else 3.0
    os.makedirs(os.path.join(out, "shots"), exist_ok=True)
    since = ssh("date '+%Y-%m-%d %H:%M:%S'").strip().splitlines()[-1]
    if "--apps" in argv:
        apps = argv[argv.index("--apps") + 1].split(",")
    else:
        apps = [a for a in ssh("ls /var/lib/springboard/apps").split() if a and not SKIP.search(a)]
    notes = {}
    for app in apps:
        name = app.removeprefix("is.olibuijr.").removesuffix(".desktop")
        rc, text = midget("open-app", app, timeout=60)
        time.sleep(settle)
        state = midget("state")[1]
        screen = next((l for l in state.splitlines() if l.startswith("screen=")), "screen=?")
        # footprint after settling: resident memory and CPU of the app's processes (the sandbox and its children)
        usage = ssh(f"for p in $(pgrep -f '/apps/{app}/'); do ps -o rss=,pcpu= -p $p; ps -o rss=,pcpu= --ppid $p; done "
                    "| awk '{r+=$1; c+=$2} END {printf \"rss=%dMB cpu=%.1f%%\", r/1024, c}'").strip().splitlines()
        screen += " " + (usage[-1] if usage else "rss=?")
        midget("shot", os.path.join(out, "shots", f"{name}.png"), "--max", "1280", timeout=60)
        notes[name] = f"open rc={rc} {screen}" + ("" if rc == 0 else f" ({text.strip().splitlines()[-1][:120] if text.strip() else ''})")
        midget("key", "cmd+q")
        time.sleep(1.0)
        print(f"{name}: {notes[name]}", flush=True)
    journal = ssh(f"journalctl -b --since '{since}' --no-pager -o short -p warning; "
                  f"journalctl -b --since '{since}' --no-pager -o short | grep -E 'springboard|bifrost|app\\[' ", timeout=180)
    lines = [l for l in journal.splitlines() if l.strip() and not NOISE.search(l)
             and (ERRWORD.search(l) or " warning" in l.lower() or "slow frame" in l)]
    lines = list(dict.fromkeys(lines))
    with open(os.path.join(out, "journal.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    slow = collections.Counter(m.group(1) for l in lines for m in [re.search(r"slow frame [\d.]+ ms screen=(\S+)", l)] if m)
    errs = collections.Counter(norm(l) for l in lines if "slow frame" not in l)
    # idle footprint of the system with every scanned app quit: two top samples 3 s apart, the second is the real one
    idle = ssh("sleep 5; top -b -n 2 -d 3 -o %CPU | awk '/^top -/{n++} n==2' | sed -n '1,4p;7,14p'; free -m | sed -n 1,2p")
    cores = ssh(f"coredumpctl list --since '{since}' --no-pager 2>/dev/null | tail -n +2").strip()
    with open(os.path.join(out, "digest.md"), "w") as f:
        f.write(f"# Midget scan {since}\n\nApps opened: {len(apps)}. Screenshots: shots/<app>.png.\n\n")
        f.write("## Core dumps\n\n" + (cores or "none") + "\n\n")
        f.write("## Distinct error/warning messages (count, most frequent first, max 40)\n\n")
        for msg, n in errs.most_common(40):
            f.write(f"- {n}x {msg[:240]}\n")
        f.write("\n## Slow frames per screen\n\n" + ("".join(f"- {s}: {n}\n" for s, n in slow.most_common()) or "none\n"))
        f.write("\n## Idle footprint (all scanned apps quit)\n\n```\n" + idle.strip() + "\n```\n")
        f.write("\n## Per app (open result, shell screen after open, footprint after settling)\n\n")
        for name, note in notes.items():
            f.write(f"- {name}: {note}\n")
    print(f"digest: {os.path.join(out, 'digest.md')} ({len(errs)} distinct messages, {sum(slow.values())} slow frames)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
