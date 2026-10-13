#!/usr/bin/env python3
"""Publish a BifrOSt PC system update: build the shell, package `springboard-shell`, add it to the lab pacman repo, serve the repo.

usage: tools/pc-publish.py [--release] [--skip-build] [--no-serve]      (also: bifrost pc publish)
The repo lives in out/pc-repo/bifrost/x86_64 and is served at http://192.168.1.12:8079/bifrost/$arch (titan's home-LAN address; Midget on VLAN 20 reaches it by routing; override with BIFROST_REPO_BIND=host:port.
Installed systems have it in /etc/pacman.conf). Machines then update with Settings > Software Update or `bifrost pc midget update` - Btrfs system generations reboot for probation; choose Install Now in Settings, or pass `--restart` to the CLI. Version = Cargo version + epoch seconds, so every publish is newer than the last.
"""
import argparse, glob, os, re, shutil, subprocess, sys, time

import pc_mirror

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POOL = os.path.join(REPO, "out/pc-repo/bifrost/x86_64")
PIDF = os.path.join(REPO, "out/pc-repo/server.pid")
_h, _, _p = os.environ.get("BIFROST_REPO_BIND", "192.168.1.12:8079").partition(":")
BIND = (_h, int(_p or 8079))


def run(cmd, **kw):
    r = subprocess.run(cmd, **kw)
    if r.returncode:
        sys.exit(f"failed: {' '.join(map(str, cmd))}")
    return r


def serve():
    if subprocess.run(["systemctl", "is-active", "--quiet", "bifrost-repo"]).returncode == 0:
        return "bifrost-repo.service"
    try:
        pid = int(open(PIDF).read())
        if os.path.exists(f"/proc/{pid}") and b"http.server" in open(f"/proc/{pid}/cmdline", "rb").read():
            return pid
    except (OSError, ValueError):
        pass
    p = subprocess.Popen([sys.executable, "-m", "http.server", str(BIND[1]), "--bind", BIND[0], "--directory", os.path.join(REPO, "out/pc-repo")],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, start_new_session=True)
    open(PIDF, "w").write(str(p.pid))
    return p.pid


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--no-serve", action="store_true")
    ap.add_argument("--skip-mirror-sync", action="store_true", help="only check the 8079 mirror, do not download")
    a = ap.parse_args()
    prof = "release" if a.release else "fast"
    t0 = time.time()
    # the 8079 mirror must hold every package the new springboard-shell depends on, or Midget's update 404s and rolls back
    if not a.skip_mirror_sync:
        try:
            pc_mirror.sync()
        except Exception as e:  # offline titan: the check below still decides
            print(f"pc-mirror: sync failed ({e}); checking what is there", file=sys.stderr)
    pc_mirror.check()
    if not a.skip_build:
        run(["cargo", "build", "--profile", prof, "--bin", "shell", "--bin", "bootweave", "--bin", "bifrost-portal", "--bin", "bifrost-privd", "--features", "desktop-wayland", "--timings"], cwd=os.path.join(REPO, "springboard"))
        run(["cargo", "build", "--profile", prof, "-p", "bifrost-mcp", "-p", "bifrostd", "-p", "bifrost-audiod", "-p", "bifrost-audio-bench", "-p", "bifrost-alsa-plugin"], cwd=os.path.join(REPO, "springboard"))
        # the window server, its chrome clients and bifrost-log (the PKGBUILD builds the same set)
        run(["cargo", "build", "--profile", prof, "-p", "bifrost-wm", "--bin", "bifrost-wm", "-p", "bifrost-chrome", "--bins", "-p", "bifrost-app", "--bin", "bifrost-log"], cwd=os.path.join(REPO, "springboard"))
    # the EFI loader is always built from source (even with --skip-build) and checked against its sources; run() exits on failure
    run([os.path.join(REPO, "tools/build-efi-boot.sh"), "release"])
    run([os.path.join(REPO, "tools/verify-efi-loader.sh"), os.path.join(REPO, "packaging/arch/recovery/bootweave.efi")])
    run(["rustc", "--edition", "2021", "-O", "-C", "target-feature=+crt-static", "-C", "strip=symbols", os.path.join(REPO,"packaging/arch/recovery/guard.rs"), "-o", os.path.join(REPO,"out/bifrost-update-guard-publish")])
    built_at = time.time()
    binpath = os.path.join(REPO, f"springboard/target/{prof}/shell")
    bootweave = os.path.join(REPO, f"springboard/target/{prof}/bootweave")
    portal = os.path.join(REPO, f"springboard/target/{prof}/bifrost-portal")
    privd = os.path.join(REPO, f"springboard/target/{prof}/bifrost-privd")
    mcp = os.path.join(REPO, f"springboard/target/{prof}/bifrost-mcp")
    bifrostd = os.path.join(REPO, f"springboard/target/{prof}/bifrostd")
    audiod = os.path.join(REPO, f"springboard/target/{prof}/bifrost-audiod")
    audio_bench = os.path.join(REPO, f"springboard/target/{prof}/bifrost-audio-bench")
    alsa_plugin = os.path.join(REPO, f"springboard/target/{prof}/libasound_module_pcm_bifrost.so")
    tdir = os.path.join(REPO, f"springboard/target/{prof}")
    wm_bins = {"PC_WM": "bifrost-wm", "PC_MENUBAR": "bifrost-menubar", "PC_DOCK": "bifrost-dock", "PC_PANELS": "bifrost-panels", "PC_LOG": "bifrost-log"}
    wm_env = {k: os.path.join(tdir, v) for k, v in wm_bins.items()}
    base = re.search(r'^version\s*=\s*"([^"]+)"', open(os.path.join(REPO, "springboard/Cargo.toml")).read(), re.M).group(1)
    ver = f"{base}.{int(time.time())}"
    work = os.path.join(REPO, "out/pc-repo/build")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    os.makedirs(POOL, exist_ok=True)
    src = os.path.join(REPO, "packaging/arch")
    for f in ("fast/PKGBUILD", "fast/springboard.install"):
        shutil.copy(os.path.join(src, f), work)
    env = {**os.environ, "PC_VER": ver, "PC_PKGSRC": src, "PC_BIN": binpath, "PC_BOOTWEAVE": bootweave, "PC_PORTAL": portal, "PC_PRIVD": privd, "PC_MCP": mcp, "PC_BIFROSTD": bifrostd, "PC_AUDIOD": audiod, "PC_AUDIO_BENCH": audio_bench, "PC_ALSA_PLUGIN": alsa_plugin, **wm_env, "PC_GUARD": os.path.join(REPO,"out/bifrost-update-guard-publish"), "PKGDEST": POOL, "BUILDDIR": work + "/b", "SRCDEST": work + "/s", "PKGEXT": ".pkg.tar.zst"}
    run(["makepkg", "-f", "--nodeps", "--skipinteg", "--nocheck"], cwd=work, env=env, stdout=subprocess.DEVNULL)
    pkg = sorted(glob.glob(os.path.join(POOL, f"springboard-shell-{ver}-1-x86_64.pkg.tar.zst")))[-1]
    # keep the last 3 packages
    olds = sorted(glob.glob(os.path.join(POOL, "springboard-shell-*.pkg.tar.zst")), key=os.path.getmtime)[:-3]
    for o in olds:
        os.unlink(o)
    run(["repo-add", "-q", "-R", os.path.join(POOL, "bifrost.db.tar.zst"), pkg], stdout=subprocess.DEVNULL)
    pid = None if a.no_serve else serve()
    print(f"build {built_at - t0:.1f}s; package/publish {time.time() - built_at:.1f}s")
    print(f"published springboard-shell {ver} ({os.path.getsize(pkg) // 1024} KiB, {prof}) in {time.time() - t0:.0f}s"
          + (f"; repo served at http://{BIND[0]}:{BIND[1]}/bifrost/x86_64 (pid {pid})" if pid else ""))


if __name__ == "__main__":
    main()
